//! The real-time half of the engine.
//!
//! Rules for everything in this file reachable from [`Renderer::process`]:
//! no allocation or deallocation, no locks, no blocking calls, no I/O, and
//! bounded work per block. `tests/realtime_safety.rs` enforces the
//! allocation rule.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use rtrb::{Consumer, Producer};

use crate::engine::{Command, EngineConfig, Event, Garbage, PlayerId, Shared};
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

/// Linear ramp used to smooth gain changes.
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

    /// Jumps straight to `target` with no ramp (for when nothing is
    /// audible, so there is no click to smooth).
    fn jump(&mut self, target: f32) {
        self.target = target;
        self.current = target;
        self.remaining = 0;
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

#[derive(Clone, Copy)]
struct Transport {
    playing: bool,
    position: Samples,
    /// Markers at this exact position have already been handled (or were
    /// landed on by a jump, seek or resume) and must not fire again until
    /// the playhead moves.
    handled_at: Option<Samples>,
}

impl Transport {
    const STOPPED: Self = Self {
        playing: false,
        position: 0,
        handled_at: None,
    };
}

/// Result of handling markers at the playhead.
enum MarkerFlow {
    /// Nothing moved; render onward.
    Continue,
    /// The playhead jumped or playback halted; re-evaluate.
    Changed,
}

/// Sends events without blocking; counts drops instead.
struct Outbox<'a> {
    events: &'a mut Producer<Event>,
    shared: &'a Shared,
}

impl Outbox<'_> {
    fn emit(&mut self, event: Event) {
        if self.events.push(event).is_err() {
            self.shared.dropped_events.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// One timeline with its own transport, gain and fade.
struct Player {
    id: PlayerId,
    timeline: Option<Box<LoadedTimeline>>,
    transport: Transport,
    gain: GainRamp,
    fade_out: Option<FadeOut>,
}

impl Player {
    fn new(id: PlayerId, ramp_frames: u32) -> Self {
        Self {
            id,
            timeline: None,
            transport: Transport::STOPPED,
            gain: GainRamp::new(1.0, ramp_frames),
            fade_out: None,
        }
    }

    fn play(&mut self, out: &mut Outbox) {
        if !self.transport.playing && self.timeline.is_some() {
            self.transport.playing = true;
            // Starting on a marker must not fire it.
            self.transport.handled_at = Some(self.transport.position);
            out.emit(Event::Started {
                player: self.id,
                position: self.transport.position,
            });
        }
    }

    fn pause(&mut self, out: &mut Outbox) {
        if self.transport.playing {
            self.transport.playing = false;
            self.fade_out = None;
            out.emit(Event::Paused {
                player: self.id,
                position: self.transport.position,
            });
        }
    }

    fn stop(&mut self, out: &mut Outbox) {
        self.halt_and_rewind();
        out.emit(Event::Stopped { player: self.id });
    }

    fn seek(&mut self, target: Samples, out: &mut Outbox) {
        if let Some(tl) = self.timeline.as_deref_mut() {
            let position = target.min(tl.timeline.length());
            tl.reset_loops();
            self.transport.position = position;
            self.transport.handled_at = Some(position);
            out.emit(Event::Seeked {
                player: self.id,
                position,
            });
        }
    }

    fn restart(&mut self, out: &mut Outbox) {
        if self.timeline.is_some() {
            self.halt_and_rewind();
            self.play(out);
        }
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

    fn halt_and_rewind(&mut self) {
        self.transport = Transport::STOPPED;
        self.fade_out = None;
        if let Some(tl) = self.timeline.as_deref_mut() {
            tl.reset_loops();
        }
    }

    fn jump(&mut self, to: Samples) {
        self.transport.position = to;
        // Markers at the landing spot do not fire; this also guarantees a
        // jump can never re-trigger itself without the playhead advancing.
        self.transport.handled_at = Some(to);
    }

    /// Renders the player into `out` (zeroed, interleaved, `ch` channels).
    fn render(&mut self, out: &mut [f32], ch: usize, outbox: &mut Outbox) {
        let frames = out.len() / ch;
        let mut frame = 0;

        while frame < frames && self.transport.playing {
            // Take the timeline out so `self` can be borrowed mutably.
            // Moving a `Box` does not allocate.
            let Some(mut tl) = self.timeline.take() else {
                self.transport.playing = false;
                break;
            };

            let pos = self.transport.position;
            if self.transport.handled_at != Some(pos)
                && let MarkerFlow::Changed = self.fire_markers(&mut tl, outbox)
            {
                self.timeline = Some(tl);
                continue;
            }

            let length = tl.timeline.length();
            if pos >= length {
                self.timeline = Some(tl);
                self.halt_and_rewind();
                outbox.emit(Event::ReachedEnd { player: self.id });
                break;
            }

            let next_stop = tl.timeline.next_marker_after(pos).unwrap_or(length);
            let mut len = ((frames - frame) as Samples).min(next_stop - pos);
            if let Some(f) = self.fade_out {
                len = len.min(f.remaining);
            }

            if len > 0 {
                let span = &mut out[frame * ch..(frame + len as usize) * ch];
                mix_timeline(&tl.timeline, pos, span, ch);
                self.apply_gain(span, ch);
                self.transport.position = pos + len;
                self.transport.handled_at = None;
                frame += len as usize;
            }
            self.timeline = Some(tl);

            if self.fade_out.is_some_and(|f| f.remaining == 0) {
                self.stop(outbox);
            }
        }
    }

    /// Handles every marker at the playhead, in order, until one moves the
    /// playhead or halts playback.
    fn fire_markers(&mut self, tl: &mut LoadedTimeline, outbox: &mut Outbox) -> MarkerFlow {
        let pos = self.transport.position;
        for i in tl.timeline.markers_at(pos) {
            let marker = &tl.timeline.markers()[i];
            let (id, action) = (marker.id, marker.action);
            outbox.emit(Event::MarkerReached {
                player: self.id,
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
                    outbox.emit(Event::Paused {
                        player: self.id,
                        position: pos,
                    });
                    return MarkerFlow::Changed;
                }
                MarkerAction::Stop => {
                    // `self.timeline` is taken while markers fire, so reset
                    // this timeline's loops directly.
                    self.halt_and_rewind();
                    tl.reset_loops();
                    outbox.emit(Event::Stopped { player: self.id });
                    return MarkerFlow::Changed;
                }
                MarkerAction::FadeOut { length } => self.start_fade_out(length),
            }
        }
        self.transport.handled_at = Some(pos);
        MarkerFlow::Continue
    }

    /// Applies player gain and any fade-out to rendered frames.
    fn apply_gain(&mut self, span: &mut [f32], ch: usize) {
        if self.gain.is_unity() && self.fade_out.is_none() {
            return;
        }
        for frame in span.chunks_exact_mut(ch) {
            let mut g = self.gain.next();
            if let Some(f) = self.fade_out.as_mut() {
                g *= f.remaining as f32 / f.total as f32;
                f.remaining -= 1;
            }
            for s in frame {
                *s *= g;
            }
        }
    }
}

/// Mixes timeline content for `[start, start + frames)` into `out`.
fn mix_timeline(timeline: &Timeline, start: Samples, out: &mut [f32], ch: usize) {
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
}

/// Real-time half of the engine. Owned by the audio device callback.
pub struct Renderer {
    channels: usize,
    max_block_frames: usize,
    commands: Consumer<Command>,
    events: Producer<Event>,
    garbage: Producer<Garbage>,
    shared: Arc<Shared>,
    players: Box<[Player]>,
    scratch: Box<[f32]>,
    master: GainRamp,
}

impl Renderer {
    pub(crate) fn new(
        config: EngineConfig,
        commands: Consumer<Command>,
        events: Producer<Event>,
        garbage: Producer<Garbage>,
        shared: Arc<Shared>,
    ) -> Self {
        let channels = usize::from(config.channels);
        Self {
            channels,
            max_block_frames: config.max_block_frames,
            commands,
            events,
            garbage,
            shared,
            players: (0..config.players)
                .map(|i| Player::new(PlayerId(i), config.gain_ramp_frames))
                .collect(),
            scratch: vec![0.0; config.max_block_frames * channels].into_boxed_slice(),
            master: GainRamp::new(1.0, config.gain_ramp_frames),
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
        let usable = out.len() / ch * ch;
        let Self {
            players,
            scratch,
            events,
            shared,
            master,
            max_block_frames,
            ..
        } = self;
        let mut outbox = Outbox {
            events,
            shared: shared.as_ref(),
        };

        for block in out[..usable].chunks_mut(*max_block_frames * ch) {
            for player in players.iter_mut() {
                if !player.transport.playing {
                    continue;
                }
                let scratch = &mut scratch[..block.len()];
                scratch.fill(0.0);
                player.render(scratch, ch, &mut outbox);
                for (o, s) in block.iter_mut().zip(scratch.iter()) {
                    *o += *s;
                }
            }
            if !master.is_unity() {
                for frame in block.chunks_exact_mut(ch) {
                    let g = master.next();
                    for s in frame {
                        *s *= g;
                    }
                }
            }
        }

        for (player, published) in self.players.iter().zip(self.shared.players.iter()) {
            published
                .position
                .store(player.transport.position, Ordering::Relaxed);
            published
                .playing
                .store(player.transport.playing, Ordering::Relaxed);
        }
    }

    fn drain_commands(&mut self) {
        while let Ok(command) = self.commands.pop() {
            let mut outbox = Outbox {
                events: &mut self.events,
                shared: &self.shared,
            };
            match command {
                Command::Load(id, new) => {
                    let player = &mut self.players[id.0 as usize];
                    let old = player.timeline.replace(new);
                    player.transport = Transport::STOPPED;
                    player.fade_out = None;
                    outbox.emit(Event::TimelineLoaded { player: id });
                    if let Some(old) = old {
                        self.dispose(Garbage::Timeline(old));
                    }
                }
                Command::Unload(id) => {
                    let player = &mut self.players[id.0 as usize];
                    let was_playing = player.transport.playing;
                    let old = player.timeline.take();
                    player.halt_and_rewind();
                    if was_playing {
                        outbox.emit(Event::Stopped { player: id });
                    }
                    if let Some(old) = old {
                        self.dispose(Garbage::Timeline(old));
                    }
                }
                Command::Play(id) => self.players[id.0 as usize].play(&mut outbox),
                Command::Pause(id) => self.players[id.0 as usize].pause(&mut outbox),
                Command::Stop(id) => self.players[id.0 as usize].stop(&mut outbox),
                Command::Seek(id, pos) => self.players[id.0 as usize].seek(pos, &mut outbox),
                Command::Restart(id) => self.players[id.0 as usize].restart(&mut outbox),
                Command::SetPlayerGain(id, gain) => {
                    let player = &mut self.players[id.0 as usize];
                    // Smooth changes only while audible; a stopped player
                    // must start at its new gain, not ramp from the old one.
                    if player.transport.playing {
                        player.gain.set(gain);
                    } else {
                        player.gain.jump(gain);
                    }
                }
                Command::FadeOutAndStop(id, length) => {
                    let player = &mut self.players[id.0 as usize];
                    if player.transport.playing {
                        player.start_fade_out(length);
                    }
                }
                Command::StopAll { fade } => {
                    for player in self.players.iter_mut() {
                        if !player.transport.playing {
                            continue;
                        }
                        if fade == 0 {
                            player.stop(&mut outbox);
                        } else {
                            player.start_fade_out(fade);
                        }
                    }
                }
                Command::SetMasterGain(gain) => self.master.set(gain),
            }
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
