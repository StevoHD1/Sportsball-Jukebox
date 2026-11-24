use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use rtrb::{Consumer, Producer, RingBuffer};

use crate::render::{LoadedTimeline, Renderer};
use crate::{MarkerId, Samples, Timeline};

/// Identifies one of an engine's players. Each player holds one timeline
/// with its own transport, gain and fades; all players are mixed together.
/// A sound-board cue maps to one player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlayerId(pub u32);

/// Settings for one engine instance. Each audio interface gets its own
/// engine, running at that interface's sample rate and channel count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineConfig {
    pub sample_rate: u32,
    /// Output channels, interleaved in the buffers passed to
    /// [`Renderer::process`].
    pub channels: u16,
    /// Number of players (simultaneous timelines). Fixed for the life of
    /// the engine so the audio thread never allocates.
    pub players: u32,
    /// Largest block mixed in one pass. Bigger device buffers are split
    /// into blocks of this size; it only sizes a scratch buffer.
    pub max_block_frames: usize,
    /// Capacity of the controller → renderer command queue.
    pub command_capacity: usize,
    /// Capacity of the renderer → controller event queue.
    pub event_capacity: usize,
    /// Length of the ramp used to smooth gain changes, in frames.
    pub gain_ramp_frames: u32,
    /// Length of the crossfade applied when the playhead jumps while
    /// audible (marker jumps and loops, retriggers, seeks) and of the fade
    /// on an operator stop, so none of them click. 0 = hard cuts.
    pub declick_frames: u32,
}

impl EngineConfig {
    pub fn new(sample_rate: u32, channels: u16) -> Self {
        Self {
            sample_rate,
            channels,
            players: 64,
            max_block_frames: 1024,
            command_capacity: 1024,
            event_capacity: 4096,
            // 10 ms: fast enough to feel instant, slow enough not to click.
            gain_ramp_frames: sample_rate / 100,
            // 3 ms: inaudible as a fade, long enough to remove clicks.
            declick_frames: sample_rate * 3 / 1000,
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
    /// No such player in this engine.
    InvalidPlayer(PlayerId),
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
            Self::InvalidPlayer(p) => write!(f, "player {} does not exist", p.0),
        }
    }
}

impl std::error::Error for EngineError {}

/// Something that happened on the audio thread. Events are `Copy` so sending
/// them never allocates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    TimelineLoaded {
        player: PlayerId,
    },
    Started {
        player: PlayerId,
        position: Samples,
    },
    Paused {
        player: PlayerId,
        position: Samples,
    },
    Stopped {
        player: PlayerId,
    },
    Seeked {
        player: PlayerId,
        position: Samples,
    },
    /// The playhead reached a marker. Fired for every marker reached,
    /// including loops that have finished repeating and pass through.
    MarkerReached {
        player: PlayerId,
        marker: MarkerId,
        position: Samples,
    },
    /// The playhead ran off the end of the timeline and stopped.
    ReachedEnd {
        player: PlayerId,
    },
}

pub(crate) enum Command {
    Load(PlayerId, Box<LoadedTimeline>),
    Unload(PlayerId),
    Play(PlayerId),
    Pause(PlayerId),
    Stop(PlayerId),
    Seek(PlayerId, Samples),
    Restart(PlayerId),
    SetPlayerGain(PlayerId, f32),
    /// Gain change ramped over a given number of frames (ducking).
    SetPlayerGainOver(PlayerId, f32, u32),
    /// Lets the player leave its current "until released" loop.
    Release(PlayerId),
    /// Output channels the player feeds: `first`, `count` (0 = all).
    SetRoute(PlayerId, u16, u16),
    FadeOutAndStop(PlayerId, Samples),
    StopAll {
        fade: Samples,
    },
    SetMasterGain(f32),
}

/// Heap objects the audio thread is done with, returned to be freed off the
/// real-time thread.
pub(crate) enum Garbage {
    // Held only so it is dropped on the control thread.
    #[allow(dead_code)]
    Timeline(Box<LoadedTimeline>),
}

#[derive(Debug, Default)]
pub(crate) struct PlayerShared {
    /// Playing and waiting for a release before it leaves a vamp loop.
    pub holding: AtomicBool,
    pub position: AtomicU64,
    pub playing: AtomicBool,
}

/// State the audio thread publishes for the UI to read without messaging.
#[derive(Debug)]
pub(crate) struct Shared {
    pub players: Box<[PlayerShared]>,
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
        if config.players == 0 || config.max_block_frames == 0 {
            return Err(EngineError::InvalidConfig(
                "players and block size must be non-zero",
            ));
        }
        if config.command_capacity == 0 || config.event_capacity == 0 {
            return Err(EngineError::InvalidConfig(
                "queue capacities must be non-zero",
            ));
        }
        let (cmd_tx, cmd_rx) = RingBuffer::new(config.command_capacity);
        let (evt_tx, evt_rx) = RingBuffer::new(config.event_capacity);
        // Each command produces at most one piece of garbage, and the
        // controller collects garbage before every send, so a garbage queue
        // as large as the command queue cannot overflow.
        let (gc_tx, gc_rx) = RingBuffer::new(config.command_capacity);
        let shared = Arc::new(Shared {
            players: (0..config.players)
                .map(|_| PlayerShared::default())
                .collect(),
            dropped_events: AtomicU64::new(0),
            leaked_garbage: AtomicU64::new(0),
        });

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

    fn check(&self, player: PlayerId) -> Result<(), EngineError> {
        if player.0 < self.config.players {
            Ok(())
        } else {
            Err(EngineError::InvalidPlayer(player))
        }
    }

    fn send(&mut self, command: Command) -> Result<(), EngineError> {
        self.collect_garbage();
        self.commands
            .push(command)
            .map_err(|_| EngineError::QueueFull)
    }

    fn send_to(&mut self, player: PlayerId, command: Command) -> Result<(), EngineError> {
        self.check(player)?;
        self.send(command)
    }

    /// Loads a timeline into a player, replacing what it held. The player
    /// stops and its playhead returns to the start.
    pub fn load(&mut self, player: PlayerId, timeline: Timeline) -> Result<(), EngineError> {
        self.check(player)?;
        if timeline.sample_rate() != self.config.sample_rate {
            return Err(EngineError::SampleRateMismatch {
                engine: self.config.sample_rate,
                timeline: timeline.sample_rate(),
            });
        }
        let loaded = Box::new(LoadedTimeline::new(timeline));
        self.send(Command::Load(player, loaded))
    }

    /// Empties a player.
    pub fn unload(&mut self, player: PlayerId) -> Result<(), EngineError> {
        self.send_to(player, Command::Unload(player))
    }

    /// Starts or resumes a player.
    pub fn play(&mut self, player: PlayerId) -> Result<(), EngineError> {
        self.send_to(player, Command::Play(player))
    }

    /// Halts a player, keeping its playhead where it is.
    pub fn pause(&mut self, player: PlayerId) -> Result<(), EngineError> {
        self.send_to(player, Command::Pause(player))
    }

    /// Halts a player and returns its playhead to the start.
    pub fn stop(&mut self, player: PlayerId) -> Result<(), EngineError> {
        self.send_to(player, Command::Stop(player))
    }

    /// Moves a player's playhead. Markers exactly at the new position do
    /// not fire.
    pub fn seek(&mut self, player: PlayerId, position: Samples) -> Result<(), EngineError> {
        self.send_to(player, Command::Seek(player, position))
    }

    /// Plays from the start, whether or not the player is already playing.
    /// This is what firing a sound-board cue does.
    pub fn restart(&mut self, player: PlayerId) -> Result<(), EngineError> {
        self.send_to(player, Command::Restart(player))
    }

    /// Sets a player's gain (linear), smoothed to avoid clicks.
    pub fn set_player_gain(&mut self, player: PlayerId, gain: f32) -> Result<(), EngineError> {
        self.send_to(player, Command::SetPlayerGain(player, sanitize_gain(gain)))
    }

    /// Changes a player's gain over `frames` frames (for ducking, where a
    /// slower, audible ramp is wanted).
    pub fn set_player_gain_over(
        &mut self,
        player: PlayerId,
        gain: f32,
        frames: u32,
    ) -> Result<(), EngineError> {
        self.send_to(
            player,
            Command::SetPlayerGainOver(player, sanitize_gain(gain), frames),
        )
    }

    /// Releases a player from its current "until released" loop: it plays
    /// on past the loop marker the next time it reaches it.
    pub fn release(&mut self, player: PlayerId) -> Result<(), EngineError> {
        self.send_to(player, Command::Release(player))
    }

    /// Sends a player to output channels `first..first + count` (0-based;
    /// `count` 0 = every channel). Mono audio feeds all of them; multi-
    /// channel audio maps channel by channel.
    pub fn set_route(
        &mut self,
        player: PlayerId,
        first: u16,
        count: u16,
    ) -> Result<(), EngineError> {
        self.send_to(player, Command::SetRoute(player, first, count))
    }

    /// Whether a player is playing and has an unreleased vamp loop ahead.
    pub fn is_holding(&self, player: PlayerId) -> bool {
        self.shared
            .players
            .get(player.0 as usize)
            .is_some_and(|p| p.holding.load(Ordering::Relaxed))
    }

    /// Fades a player to silence over `length` frames, then stops it.
    pub fn fade_out_and_stop(
        &mut self,
        player: PlayerId,
        length: Samples,
    ) -> Result<(), EngineError> {
        self.send_to(player, Command::FadeOutAndStop(player, length))
    }

    /// Fades every playing player out over `fade` frames (0 = immediately)
    /// and stops them. The "panic" control.
    pub fn stop_all(&mut self, fade: Samples) -> Result<(), EngineError> {
        self.send(Command::StopAll { fade })
    }

    /// Sets the master output gain (linear), smoothed to avoid clicks.
    pub fn set_master_gain(&mut self, gain: f32) -> Result<(), EngineError> {
        self.send(Command::SetMasterGain(sanitize_gain(gain)))
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

    /// A player's playhead as of the end of the last processed audio block.
    /// Returns 0 for an invalid player.
    pub fn position(&self, player: PlayerId) -> Samples {
        self.shared
            .players
            .get(player.0 as usize)
            .map_or(0, |p| p.position.load(Ordering::Relaxed))
    }

    pub fn is_playing(&self, player: PlayerId) -> bool {
        self.shared
            .players
            .get(player.0 as usize)
            .is_some_and(|p| p.playing.load(Ordering::Relaxed))
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

fn sanitize_gain(gain: f32) -> f32 {
    if gain.is_finite() {
        gain.clamp(0.0, 16.0)
    } else {
        0.0
    }
}
