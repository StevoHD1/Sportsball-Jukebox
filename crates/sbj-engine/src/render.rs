//! The real-time half of the engine.
//!
//! Rules for everything in this file reachable from [`Renderer::process`]:
//! no allocation or deallocation, no locks, no blocking calls, no I/O, and
//! bounded work per block. `tests/realtime_safety.rs` enforces the
//! allocation rule.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use rtrb::{Consumer, Producer};

use crate::engine::{Command, EngineConfig, Event, Garbage, Shared};
use crate::{LoopCount, MarkerAction, Samples, Timeline};

/// A timeline plus the per-marker runtime state the renderer mutates.
/// Built on the control thread so the audio thread never allocates.
pub(crate) struct LoadedTimeline {
    timeline: Timeline,
    /// Remaining jumps for each `Loop { count: Times(n) }` marker, indexed
    /// like `timeline.markers()`.
    loop_remaining: Vec<u32>,
}

impl LoadedTimeline {
    pub(crate) fn new(timeline: Timeline) -> Self {
        let loop_remaining = vec![0; timeline.markers().len()];
        let mut loaded = Self {
            timeline,
            loop_remaining,
        };
        loaded.reset_loops();
        loaded
    }

    fn reset_loops(&mut self) {
        for (remaining, marker) in self.loop_remaining.iter_mut().zip(self.timeline.markers()) {
            *remaining = initial_loop_count(marker.action);
        }
    }
}

fn initial_loop_count(action: MarkerAction) -> u32 {
    match action {
        MarkerAction::Loop {
            count: LoopCount::Times(n),
            ..
        } => n,
        _ => 0,
    }
}

/// Linear ramp used to smooth master gain changes.
struct GainRamp {
    current: f32,
    target: f32,
    step: f32,
    remaining: u32,
    ramp_frames: u32,
}

impl GainRamp {
    fn new(gain: f32, ramp_frames: u32) -> Self {
        Self {
            current: gain,
            target: gain,
            step: 0.0,
            remaining: 0,
            ramp_frames,
        }
    }

    fn set(&mut self, target: f32) {
        self.target = target;
        if self.ramp_frames == 0 {
            self.current = target;
            self.remaining = 0;
        } else {
            self.step = (target - self.current) / self.ramp_frames as f32;
            self.remaining = self.ramp_frames;
        }
    }

    #[inline]
    fn next(&mut self) -> f32 {
        if self.remaining > 0 {
            self.remaining -= 1;
            self.current = if self.remaining == 0 {
                self.target
            } else {
                self.current + self.step
            };
        }
        self.current
    }

    #[inline]
    fn is_unity(&self) -> bool {
        self.remaining == 0 && self.current == 1.0
    }
}

/// An in-progress fade to silence that ends in a stop.
#[derive(Clone, Copy)]
struct FadeOut {
    remaining: Samples,
    total: Samples,
}

struct Transport {
    playing: bool,
    position: Samples,
    /// Markers at this exact position have already been handled (or were
    /// landed on by a jump, seek or resume) and must not fire again until
    /// the playhead moves.
    handled_at: Option<Samples>,
}

/// Result of handling markers at the playhead.
enum MarkerFlow {
    /// Nothing moved; render onward.
    Continue,
    /// The playhead jumped or playback halted; re-evaluate.
    Changed,
}

/// Real-time half of the engine. Owned by the audio device callback.
pub struct Renderer {
    channels: usize,
    commands: Consumer<Command>,
    events: Producer<Event>,
    garbage: Producer<Garbage>,
    shared: Arc<Shared>,
    timeline: Option<Box<LoadedTimeline>>,
    transport: Transport,
    master: GainRamp,
    fade_out: Option<FadeOut>,
}

impl Renderer {
    pub(crate) fn new(
        config: EngineConfig,
        commands: Consumer<Command>,
        events: Producer<Event>,
        garbage: Producer<Garbage>,
        shared: Arc<Shared>,
    ) -> Self {
        Self {
            channels: usize::from(config.channels),
            commands,
            events,
            garbage,
            shared,
            timeline: None,
            transport: Transport {
                playing: false,
                position: 0,
                handled_at: None,
            },
            master: GainRamp::new(1.0, config.gain_ramp_frames),
            fade_out: None,
        }
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Renders the next block of audio into `out` (interleaved, length a
    /// multiple of the channel count; any trailing partial frame is
    /// silenced). Real-time safe.
    pub fn process(&mut self, out: &mut [f32]) {
        self.drain_commands();
        out.fill(0.0);

        let ch = self.channels;
        let frames = out.len() / ch;
        let mut frame = 0;

        while frame < frames && self.transport.playing {
            // Take the timeline out so helpers can borrow `self` mutably.
            // Moving a `Box` does not allocate.
            let Some(mut tl) = self.timeline.take() else {
                self.transport.playing = false;
                break;
            };

            let pos = self.transport.position;
            if self.transport.handled_at != Some(pos)
                && let MarkerFlow::Changed = self.fire_markers(&mut tl)
            {
                self.timeline = Some(tl);
                continue;
            }

            let length = tl.timeline.length();
            if pos >= length {
                self.halt_and_rewind(&mut tl);
                self.emit(Event::ReachedEnd);
                self.timeline = Some(tl);
                break;
            }

            let next_stop = tl.timeline.next_marker_after(pos).unwrap_or(length);
            let mut len = (frames - frame) as Samples;
            len = len.min(next_stop - pos);
            if let Some(f) = self.fade_out {
                len = len.min(f.remaining);
            }

            if len > 0 {
                let span = &mut out[frame * ch..(frame + len as usize) * ch];
                self.render_span(&tl.timeline, pos, span);
                self.transport.position = pos + len;
                self.transport.handled_at = None;
                frame += len as usize;
            }

            if self.fade_out.is_some_and(|f| f.remaining == 0) {
                self.halt_and_rewind(&mut tl);
                self.emit(Event::Stopped);
            }
            self.timeline = Some(tl);
        }

        self.shared
            .position
            .store(self.transport.position, Ordering::Relaxed);
        self.shared
            .playing
            .store(self.transport.playing, Ordering::Relaxed);
    }

    fn drain_commands(&mut self) {
        while let Ok(command) = self.commands.pop() {
            match command {
                Command::SetTimeline(new) => {
                    if let Some(old) = self.timeline.replace(new) {
                        self.dispose(Garbage::Timeline(old));
                    }
                    self.transport = Transport {
                        playing: false,
                        position: 0,
                        handled_at: None,
                    };
                    self.fade_out = None;
                    self.emit(Event::TimelineLoaded);
                }
                Command::Play => {
                    if !self.transport.playing && self.timeline.is_some() {
                        self.transport.playing = true;
                        // Starting on a marker must not fire it.
                        self.transport.handled_at = Some(self.transport.position);
                        self.emit(Event::Started {
                            position: self.transport.position,
                        });
                    }
                }
                Command::Pause => {
                    if self.transport.playing {
                        self.transport.playing = false;
                        self.fade_out = None;
                        self.emit(Event::Paused {
                            position: self.transport.position,
                        });
                    }
                }
                Command::Stop => {
                    if let Some(mut tl) = self.timeline.take() {
                        self.halt_and_rewind(&mut tl);
                        self.timeline = Some(tl);
                    }
                    self.emit(Event::Stopped);
                }
                Command::Seek(target) => {
                    if let Some(tl) = self.timeline.as_deref_mut() {
                        let position = target.min(tl.timeline.length());
                        tl.reset_loops();
                        self.transport.position = position;
                        self.transport.handled_at = Some(position);
                        self.emit(Event::Seeked { position });
                    }
                }
                Command::SetMasterGain(gain) => self.master.set(gain),
                Command::FadeOutAndStop(length) => {
                    if self.transport.playing {
                        self.start_fade_out(length);
                    }
                }
            }
        }
    }

    /// Handles every marker at the playhead, in order, until one moves the
    /// playhead or halts playback.
    fn fire_markers(&mut self, tl: &mut LoadedTimeline) -> MarkerFlow {
        let pos = self.transport.position;
        for i in tl.timeline.markers_at(pos) {
            let marker = &tl.timeline.markers()[i];
            let (id, action) = (marker.id, marker.action);
            self.emit(Event::MarkerReached {
                marker: id,
                position: pos,
            });
            match action {
                MarkerAction::Jump { to } => {
                    self.jump(to);
                    return MarkerFlow::Changed;
                }
                MarkerAction::Loop { to, count } => {
                    let jump = match count {
                        LoopCount::Infinite => true,
                        LoopCount::Times(n) => {
                            let remaining = &mut tl.loop_remaining[i];
                            if *remaining > 0 {
                                *remaining -= 1;
                                true
                            } else {
                                // Passing through: re-arm for next time.
                                *remaining = n;
                                false
                            }
                        }
                    };
                    if jump {
                        self.jump(to);
                        return MarkerFlow::Changed;
                    }
                }
                MarkerAction::Pause => {
                    self.transport.playing = false;
                    self.transport.handled_at = Some(pos);
                    self.fade_out = None;
                    self.emit(Event::Paused { position: pos });
                    return MarkerFlow::Changed;
                }
                MarkerAction::Stop => {
                    self.halt_and_rewind(tl);
                    self.emit(Event::Stopped);
                    return MarkerFlow::Changed;
                }
                MarkerAction::FadeOut { length } => self.start_fade_out(length),
            }
        }
        self.transport.handled_at = Some(pos);
        MarkerFlow::Continue
    }

    fn jump(&mut self, to: Samples) {
        self.transport.position = to;
        // Markers at the landing spot do not fire; this also guarantees a
        // jump can never re-trigger itself without the playhead advancing.
        self.transport.handled_at = Some(to);
    }

    fn start_fade_out(&mut self, length: Samples) {
        // Never lengthen a fade that is already running.
        let remaining = match self.fade_out {
            Some(f) => f.remaining.min(length),
            None => length,
        };
        self.fade_out = Some(FadeOut {
            remaining,
            total: remaining.max(1),
        });
    }

    fn halt_and_rewind(&mut self, tl: &mut LoadedTimeline) {
        self.transport.playing = false;
        self.transport.position = 0;
        self.transport.handled_at = None;
        self.fade_out = None;
        tl.reset_loops();
    }

    /// Mixes timeline content for `[start, start + frames)` into `out`,
    /// which must already be zeroed, then applies master gain and fades.
    fn render_span(&mut self, timeline: &Timeline, start: Samples, out: &mut [f32]) {
        let ch = self.channels;
        let frames = (out.len() / ch) as Samples;
        let end = start + frames;

        let clips = timeline.clips();
        let candidates = clips.partition_point(|c| c.start < end);
        for clip in &clips[..candidates] {
            if clip.end() <= start {
                continue;
            }
            let from = start.max(clip.start);
            let to = end.min(clip.end());
            let data = clip.buffer.data();
            let src_ch = usize::from(clip.buffer.channels());

            for t in from..to {
                let offset = t - clip.start;
                let gain = clip.gain * clip.envelope(offset);
                let src = (clip.source_offset + offset) as usize * src_ch;
                let dst = (t - start) as usize * ch;
                if src_ch == 1 {
                    let s = data[src] * gain;
                    for o in &mut out[dst..dst + ch] {
                        *o += s;
                    }
                } else {
                    for c in 0..ch.min(src_ch) {
                        out[dst + c] += data[src + c] * gain;
                    }
                }
            }
        }

        if self.master.is_unity() && self.fade_out.is_none() {
            return;
        }
        for frame in out.chunks_exact_mut(ch) {
            let mut g = self.master.next();
            if let Some(f) = self.fade_out.as_mut() {
                g *= f.remaining as f32 / f.total as f32;
                f.remaining -= 1;
            }
            for s in frame {
                *s *= g;
            }
        }
    }

    fn emit(&mut self, event: Event) {
        if self.events.push(event).is_err() {
            self.shared.dropped_events.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn dispose(&mut self, garbage: Garbage) {
        if let Err(rtrb::PushError::Full(garbage)) = self.garbage.push(garbage) {
            // Freeing here would break real-time safety; leaking is the
            // lesser evil and is reported via `Controller::leaked_garbage`.
            std::mem::forget(garbage);
            self.shared.leaked_garbage.fetch_add(1, Ordering::Relaxed);
        }
    }
}
