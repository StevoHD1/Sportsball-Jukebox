use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use rtrb::{Consumer, Producer, RingBuffer};

use crate::render::{LoadedTimeline, Renderer};
use crate::{MarkerId, Samples, Timeline};

/// Settings for one engine instance. Each audio interface gets its own
/// engine, running at that interface's sample rate and channel count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineConfig {
    pub sample_rate: u32,
    /// Output channels, interleaved in the buffers passed to
    /// [`Renderer::process`].
    pub channels: u16,
    /// Capacity of the controller → renderer command queue.
    pub command_capacity: usize,
    /// Capacity of the renderer → controller event queue.
    pub event_capacity: usize,
    /// Length of the ramp used to smooth master gain changes, in frames.
    pub gain_ramp_frames: u32,
}

impl EngineConfig {
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            sample_rate,
            channels,
            command_capacity: 1024,
            event_capacity: 4096,
            // 10 ms: fast enough to feel instant, slow enough not to click.
            gain_ramp_frames: sample_rate / 100,
        }
    }
}

/// Errors reported on the control side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineError {
    InvalidConfig(&'static str),
    /// The command queue is full: the audio thread is not running or has
    /// fallen far behind. The command was not sent.
    QueueFull,
    /// The timeline's sample rate does not match the engine's.
    SampleRateMismatch {
        engine: u32,
        timeline: u32,
    },
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(why) => write!(f, "invalid engine config: {why}"),
            Self::QueueFull => write!(f, "engine command queue is full"),
            Self::SampleRateMismatch { engine, timeline } => write!(
                f,
                "timeline is {timeline} Hz but the engine runs at {engine} Hz"
            ),
        }
    }
}

impl std::error::Error for EngineError {}

/// Something that happened on the audio thread. Events are `Copy` so sending
/// them never allocates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    TimelineLoaded,
    Started {
        position: Samples,
    },
    Paused {
        position: Samples,
    },
    Stopped,
    Seeked {
        position: Samples,
    },
    /// The playhead reached a marker. Fired for every marker reached,
    /// including loops that have finished repeating and pass through.
    MarkerReached {
        marker: MarkerId,
        position: Samples,
    },
    /// The playhead ran off the end of the timeline and stopped.
    ReachedEnd,
}

pub(crate) enum Command {
    SetTimeline(Box<LoadedTimeline>),
    Play,
    Pause,
    Stop,
    Seek(Samples),
    SetMasterGain(f32),
    FadeOutAndStop(Samples),
}

/// Heap objects the audio thread is done with, returned to be freed off the
/// real-time thread.
pub(crate) enum Garbage {
    // Held only so it is dropped on the control thread.
    #[allow(dead_code)]
    Timeline(Box<LoadedTimeline>),
}

/// State the audio thread publishes for the UI to read without messaging.
#[derive(Debug, Default)]
pub(crate) struct Shared {
    pub position: AtomicU64,
    pub playing: AtomicBool,
    pub dropped_events: AtomicU64,
    pub leaked_garbage: AtomicU64,
}

/// Creates engine instances.
pub struct Engine;

impl Engine {
    /// Creates a connected controller / renderer pair. Move the renderer to
    /// the audio thread; keep the controller on the control thread.
    pub fn create(config: EngineConfig) -> Result<(Controller, Renderer), EngineError> {
        if config.sample_rate == 0 {
            return Err(EngineError::InvalidConfig("sample rate must be non-zero"));
        }
        if config.channels == 0 {
            return Err(EngineError::InvalidConfig("channels must be non-zero"));
        }
        if config.command_capacity == 0 || config.event_capacity == 0 {
            return Err(EngineError::InvalidConfig(
                "queue capacities must be non-zero",
            ));
        }
        let (cmd_tx, cmd_rx) = RingBuffer::new(config.command_capacity);
        let (evt_tx, evt_rx) = RingBuffer::new(config.event_capacity);
        // Each command produces at most one piece of garbage, so a garbage
        // queue as large as the command queue cannot overflow as long as the
        // controller collects before sending (which it always does).
        let (gc_tx, gc_rx) = RingBuffer::new(config.command_capacity);
        let shared = Arc::new(Shared::default());

        let controller = Controller {
            config,
            commands: cmd_tx,
            events: evt_rx,
            garbage: gc_rx,
            shared: Arc::clone(&shared),
        };
        let renderer = Renderer::new(config, cmd_rx, evt_tx, gc_tx, shared);
        Ok((controller, renderer))
    }
}

/// Control-thread handle to an engine.
pub struct Controller {
    config: EngineConfig,
    commands: Producer<Command>,
    events: Consumer<Event>,
    garbage: Consumer<Garbage>,
    shared: Arc<Shared>,
}

impl Controller {
    pub fn config(&self) -> EngineConfig {
        self.config
    }

    fn send(&mut self, command: Command) -> Result<(), EngineError> {
        self.collect_garbage();
        self.commands
            .push(command)
            .map_err(|_| EngineError::QueueFull)
    }

    /// Replaces the current timeline. Playback pauses and the playhead
    /// returns to the start.
    pub fn load_timeline(&mut self, timeline: Timeline) -> Result<(), EngineError> {
        if timeline.sample_rate() != self.config.sample_rate {
            return Err(EngineError::SampleRateMismatch {
                engine: self.config.sample_rate,
                timeline: timeline.sample_rate(),
            });
        }
        let loaded = Box::new(LoadedTimeline::new(timeline));
        self.send(Command::SetTimeline(loaded))
    }

    pub fn play(&mut self) -> Result<(), EngineError> {
        self.send(Command::Play)
    }

    /// Halts playback, keeping the playhead where it is.
    pub fn pause(&mut self) -> Result<(), EngineError> {
        self.send(Command::Pause)
    }

    /// Halts playback and returns the playhead to the start.
    pub fn stop(&mut self) -> Result<(), EngineError> {
        self.send(Command::Stop)
    }

    /// Moves the playhead. Markers exactly at the new position do not fire.
    pub fn seek(&mut self, position: Samples) -> Result<(), EngineError> {
        self.send(Command::Seek(position))
    }

    /// Sets the master output gain (linear), smoothed to avoid clicks.
    pub fn set_master_gain(&mut self, gain: f32) -> Result<(), EngineError> {
        let gain = if gain.is_finite() { gain.max(0.0) } else { 0.0 };
        self.send(Command::SetMasterGain(gain))
    }

    /// Fades the output to silence over `length` frames, then stops.
    pub fn fade_out_and_stop(&mut self, length: Samples) -> Result<(), EngineError> {
        self.send(Command::FadeOutAndStop(length))
    }

    /// Next event from the audio thread, if any.
    pub fn poll_event(&mut self) -> Option<Event> {
        self.events.pop().ok()
    }

    /// Frees objects the audio thread has finished with. Called
    /// automatically before each command; the UI should also call it
    /// periodically (e.g. on its refresh timer). Returns how many were freed.
    pub fn collect_garbage(&mut self) -> usize {
        let mut n = 0;
        while self.garbage.pop().is_ok() {
            n += 1;
        }
        n
    }

    /// Playhead position as of the end of the last processed audio block.
    pub fn position(&self) -> Samples {
        self.shared.position.load(Ordering::Relaxed)
    }

    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::Relaxed)
    }

    /// Events dropped because the event queue was full. Should stay zero;
    /// a non-zero value means the UI is not polling often enough.
    pub fn dropped_events(&self) -> u64 {
        self.shared.dropped_events.load(Ordering::Relaxed)
    }

    /// Objects leaked (rather than freed on the audio thread) because the
    /// garbage queue was full. Should always be zero.
    pub fn leaked_garbage(&self) -> u64 {
        self.shared.leaked_garbage.load(Ordering::Relaxed)
    }
}
