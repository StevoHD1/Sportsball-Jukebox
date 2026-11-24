//! The app model: a bank of cue slots playing through one output device.
//!
//! This is what the UI drives (through `sbj-ffi`). It owns the audio
//! backend, the open output stream, and the engine controller, and maps
//! cues to engine players one-to-one.
//!
//! Phase 1 scope: one output device at a time. Several devices at once
//! (one engine each) comes in Phase 2.

use std::fmt;
use std::path::Path;
use std::sync::Arc;

use sbj_backend::{
    AudioBackend, BackendError, DeviceId, DeviceInfo, OutputStream, StreamConfig, StreamHealth,
};
use sbj_engine::{
    AudioBuffer, Clip, Controller, Engine, EngineConfig, EngineError, LoopCount, Marker,
    MarkerAction, PlayerId, Timeline, TimelineError, samples_to_seconds, seconds_to_samples,
};
use sbj_media::MediaError;

#[derive(Debug)]
pub enum SessionError {
    InvalidCue(u32),
    NoOutput,
    Backend(BackendError),
    Engine(EngineError),
    Media(MediaError),
    Timeline(TimelineError),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCue(i) => write!(f, "cue {i} does not exist"),
            Self::NoOutput => write!(f, "no audio output is open"),
            Self::Backend(e) => e.fmt(f),
            Self::Engine(e) => e.fmt(f),
            Self::Media(e) => e.fmt(f),
            Self::Timeline(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for SessionError {}

impl From<BackendError> for SessionError {
    fn from(e: BackendError) -> Self {
        Self::Backend(e)
    }
}
impl From<EngineError> for SessionError {
    fn from(e: EngineError) -> Self {
        Self::Engine(e)
    }
}
impl From<MediaError> for SessionError {
    fn from(e: MediaError) -> Self {
        Self::Media(e)
    }
}
impl From<TimelineError> for SessionError {
    fn from(e: TimelineError) -> Self {
        Self::Timeline(e)
    }
}

pub type Result<T> = std::result::Result<T, SessionError>;

/// A decoded sound, ready to assign to cues. Decoding is slow, so do it on
/// a background thread; passing the output's sample rate as
/// `target_rate` also does the resampling there, off the session lock.
#[derive(Debug, Clone)]
pub struct Sound {
    original: Arc<AudioBuffer>,
    converted: Option<Arc<AudioBuffer>>,
}

impl Sound {
    pub fn load(path: impl AsRef<Path>, target_rate: Option<u32>) -> Result<Self> {
        Self::from_buffer(sbj_media::decode_file(path)?, target_rate)
    }

    pub fn from_buffer(buffer: AudioBuffer, target_rate: Option<u32>) -> Result<Self> {
        let original = Arc::new(buffer);
        let converted = match target_rate {
            Some(rate) if rate != original.sample_rate() => {
                Some(Arc::new(sbj_media::resample(&original, rate)?))
            }
            _ => None,
        };
        Ok(Self {
            original,
            converted,
        })
    }

    pub fn duration_seconds(&self) -> f64 {
        samples_to_seconds(self.original.frames(), self.original.sample_rate())
    }

    pub fn channels(&self) -> u16 {
        self.original.channels()
    }

    pub fn sample_rate(&self) -> u32 {
        self.original.sample_rate()
    }

    /// The audio at `rate`, resampling (slow) only if not already prepared.
    fn at_rate(&self, rate: u32) -> Result<Arc<AudioBuffer>> {
        if self.original.sample_rate() == rate {
            return Ok(Arc::clone(&self.original));
        }
        match &self.converted {
            Some(c) if c.sample_rate() == rate => Ok(Arc::clone(c)),
            _ => Ok(Arc::new(sbj_media::resample(&self.original, rate)?)),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Cue {
    sound: Option<Sound>,
    /// The sound at the current output rate, cached.
    prepared: Option<Arc<AudioBuffer>>,
    /// Loop setting of the timeline loaded in the engine, if any.
    loaded_looping: Option<bool>,
    looping: bool,
    gain: f32,
}

/// Snapshot of a cue for display.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CueState {
    pub assigned: bool,
    pub playing: bool,
    pub looping: bool,
    pub gain: f32,
    pub position_seconds: f64,
    pub duration_seconds: f64,
}

/// Snapshot of the output for display.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputState {
    pub device: DeviceId,
    pub sample_rate: u32,
    pub channels: u16,
    pub health: StreamHealth,
    /// Device-reported buffer underruns (audible glitches) since opening.
    pub xruns: u64,
    /// Engine events dropped because nobody polled. Should be 0.
    pub dropped_events: u64,
}

struct Output<S> {
    device: DeviceId,
    config: StreamConfig,
    controller: Controller,
    stream: S,
}

pub struct Session<B: AudioBackend> {
    backend: B,
    cues: Vec<Cue>,
    output: Option<Output<B::Stream>>,
    master_gain: f32,
}

impl<B: AudioBackend> Session<B> {
    pub fn new(backend: B, cue_count: u32) -> Self {
        Self {
            backend,
            cues: (0..cue_count)
                .map(|_| Cue {
                    gain: 1.0,
                    ..Cue::default()
                })
                .collect(),
            output: None,
            master_gain: 1.0,
        }
    }

    pub fn cue_count(&self) -> u32 {
        self.cues.len() as u32
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn devices(&self) -> Result<Vec<DeviceInfo>> {
        Ok(self.backend.output_devices()?)
    }

    /// Opens `device` (or the system default) as the output, replacing any
    /// open output. Every assigned cue is converted to the device's rate,
    /// which can take a moment with many long sounds.
    pub fn open_output(&mut self, device: Option<&DeviceId>) -> Result<()> {
        self.close_output();
        let device = match device {
            Some(d) => d.clone(),
            None => self.backend.default_output_device()?,
        };
        let config = self.backend.preferred_config(&device)?;

        let mut engine_config = EngineConfig::new(config.sample_rate, config.channels);
        engine_config.players = self.cue_count().max(1);
        let (mut controller, mut renderer) = Engine::create(engine_config)?;
        controller.set_master_gain(self.master_gain)?;

        for (i, cue) in self.cues.iter_mut().enumerate() {
            cue.prepared = None;
            cue.loaded_looping = None;
            if let Some(sound) = &cue.sound {
                let buffer = sound.at_rate(config.sample_rate)?;
                controller.load(PlayerId(i as u32), cue_timeline(&buffer, cue.looping)?)?;
                controller.set_player_gain(PlayerId(i as u32), cue.gain)?;
                cue.prepared = Some(buffer);
                cue.loaded_looping = Some(cue.looping);
            }
        }

        let stream = self.backend.open_output(
            &device,
            config,
            Box::new(move |out| renderer.process(out)),
        )?;
        self.output = Some(Output {
            device,
            config,
            controller,
            stream,
        });
        Ok(())
    }

    pub fn close_output(&mut self) {
        self.output = None;
        for cue in &mut self.cues {
            cue.prepared = None;
            cue.loaded_looping = None;
        }
    }

    pub fn output_state(&self) -> Option<OutputState> {
        self.output.as_ref().map(|o| OutputState {
            device: o.device.clone(),
            sample_rate: o.config.sample_rate,
            channels: o.config.channels,
            health: o.stream.status().health(),
            xruns: o.stream.status().xruns(),
            dropped_events: o.controller.dropped_events(),
        })
    }

    /// The open stream, e.g. to pull audio from an offline backend in tests.
    pub fn output_stream_mut(&mut self) -> Option<&mut B::Stream> {
        self.output.as_mut().map(|o| &mut o.stream)
    }

    fn cue_mut(&mut self, cue: u32) -> Result<&mut Cue> {
        self.cues
            .get_mut(cue as usize)
            .ok_or(SessionError::InvalidCue(cue))
    }

    fn controller(&mut self) -> Result<&mut Controller> {
        self.output
            .as_mut()
            .map(|o| &mut o.controller)
            .ok_or(SessionError::NoOutput)
    }

    /// Loads the cue's current sound and loop setting into its player.
    fn reload(&mut self, cue: u32) -> Result<()> {
        let Some(output) = self.output.as_mut() else {
            return Ok(());
        };
        let rate = output.config.sample_rate;
        let c = self
            .cues
            .get_mut(cue as usize)
            .ok_or(SessionError::InvalidCue(cue))?;
        let player = PlayerId(cue);
        match &c.sound {
            None => {
                c.prepared = None;
                c.loaded_looping = None;
                output.controller.unload(player)?;
            }
            Some(sound) => {
                let buffer = match &c.prepared {
                    Some(b) if b.sample_rate() == rate => Arc::clone(b),
                    _ => sound.at_rate(rate)?,
                };
                output
                    .controller
                    .load(player, cue_timeline(&buffer, c.looping)?)?;
                output.controller.set_player_gain(player, c.gain)?;
                c.prepared = Some(buffer);
                c.loaded_looping = Some(c.looping);
            }
        }
        Ok(())
    }

    /// Assigns a sound to a cue. If the cue was playing, it stops.
    pub fn set_cue_sound(&mut self, cue: u32, sound: Sound) -> Result<()> {
        let c = self.cue_mut(cue)?;
        c.sound = Some(sound);
        c.prepared = None;
        self.reload(cue)
    }

    pub fn clear_cue(&mut self, cue: u32) -> Result<()> {
        let c = self.cue_mut(cue)?;
        c.sound = None;
        c.prepared = None;
        self.reload(cue)
    }

    /// Turns looping on or off. Takes effect immediately if the cue is
    /// stopped, or the next time it is fired if it is playing.
    pub fn set_cue_loop(&mut self, cue: u32, looping: bool) -> Result<()> {
        let c = self.cue_mut(cue)?;
        if c.looping == looping {
            return Ok(());
        }
        c.looping = looping;
        let playing = self
            .output
            .as_ref()
            .is_some_and(|o| o.controller.is_playing(PlayerId(cue)));
        if playing { Ok(()) } else { self.reload(cue) }
    }

    pub fn set_cue_gain(&mut self, cue: u32, gain: f32) -> Result<()> {
        self.cue_mut(cue)?.gain = gain;
        if let Some(output) = self.output.as_mut() {
            output.controller.set_player_gain(PlayerId(cue), gain)?;
        }
        Ok(())
    }

    /// Plays a cue from the start (restarting it if already playing).
    /// Firing an empty cue does nothing.
    pub fn fire_cue(&mut self, cue: u32) -> Result<()> {
        let c = self.cue_mut(cue)?;
        if c.sound.is_none() {
            return Ok(());
        }
        let stale = c.loaded_looping != Some(c.looping);
        if self.output.is_none() {
            return Err(SessionError::NoOutput);
        }
        if stale {
            // A loop change made while the cue was playing.
            self.reload(cue)?;
        }
        self.controller()?.restart(PlayerId(cue))?;
        Ok(())
    }

    /// Stops a cue, fading out over `fade_seconds` (0 = immediately).
    pub fn stop_cue(&mut self, cue: u32, fade_seconds: f64) -> Result<()> {
        self.cue_mut(cue)?;
        let rate = self.output_rate()?;
        let fade = seconds_to_samples(fade_seconds, rate);
        let controller = self.controller()?;
        if fade == 0 {
            controller.stop(PlayerId(cue))?;
        } else {
            controller.fade_out_and_stop(PlayerId(cue), fade)?;
        }
        Ok(())
    }

    /// Stops everything, fading out over `fade_seconds` (0 = immediately).
    pub fn stop_all(&mut self, fade_seconds: f64) -> Result<()> {
        let rate = self.output_rate()?;
        let fade = seconds_to_samples(fade_seconds, rate);
        self.controller()?.stop_all(fade)?;
        Ok(())
    }

    pub fn set_master_gain(&mut self, gain: f32) -> Result<()> {
        self.master_gain = gain;
        if let Some(output) = self.output.as_mut() {
            output.controller.set_master_gain(gain)?;
        }
        Ok(())
    }

    pub fn master_gain(&self) -> f32 {
        self.master_gain
    }

    fn output_rate(&self) -> Result<u32> {
        self.output
            .as_ref()
            .map(|o| o.config.sample_rate)
            .ok_or(SessionError::NoOutput)
    }

    pub fn cue_state(&self, cue: u32) -> Result<CueState> {
        let c = self
            .cues
            .get(cue as usize)
            .ok_or(SessionError::InvalidCue(cue))?;
        let (playing, position_seconds) = match &self.output {
            Some(o) => (
                o.controller.is_playing(PlayerId(cue)),
                samples_to_seconds(o.controller.position(PlayerId(cue)), o.config.sample_rate),
            ),
            None => (false, 0.0),
        };
        Ok(CueState {
            assigned: c.sound.is_some(),
            playing,
            looping: c.looping,
            gain: c.gain,
            position_seconds,
            duration_seconds: c.sound.as_ref().map_or(0.0, Sound::duration_seconds),
        })
    }

    /// Housekeeping: drains engine events and frees retired audio. Call
    /// regularly from the UI (e.g. on its refresh timer).
    pub fn poll(&mut self) {
        if let Some(output) = self.output.as_mut() {
            while output.controller.poll_event().is_some() {}
            output.controller.collect_garbage();
        }
    }
}

/// A cue's timeline: the whole sound, optionally looping forever.
fn cue_timeline(buffer: &Arc<AudioBuffer>, looping: bool) -> Result<Timeline> {
    let mut builder = Timeline::builder(buffer.sample_rate()).clip(Clip::new(Arc::clone(buffer)));
    if looping {
        builder = builder.marker(Marker::new(
            0,
            buffer.frames(),
            MarkerAction::Loop {
                to: 0,
                count: LoopCount::Infinite,
            },
        ));
    }
    Ok(builder.build()?)
}
