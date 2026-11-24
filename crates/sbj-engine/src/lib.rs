//! Real-time playback engine for Sportsball Jukebox.
//!
//! The engine is split in two halves that talk through lock-free queues:
//!
//! * [`Controller`] lives on the UI / control thread. It builds and validates
//!   timelines, sends commands and receives events. It may allocate freely.
//! * [`Renderer`] lives on the audio device's real-time thread. Its
//!   [`Renderer::process`] method never allocates, frees, locks or blocks.
//!
//! Anything that needs to be freed after leaving the audio thread (for
//! example a timeline that has just been replaced) is handed back to the
//! controller through a garbage queue and dropped there.
//!
//! All positions are expressed in sample frames ([`Samples`]) at the
//! engine's sample rate, which is what makes marker actions sample-accurate.
//!
//! See `docs/architecture.md` in the repository for the full design.

mod buffer;
mod engine;
mod render;
mod timeline;

pub use buffer::{AudioBuffer, BufferError};
pub use engine::{Controller, Engine, EngineConfig, EngineError, Event};
pub use render::Renderer;
pub use timeline::{
    Clip, Fade, FadeCurve, LoopCount, Marker, MarkerAction, MarkerId, Timeline, TimelineBuilder,
    TimelineError,
};

/// A position or duration measured in sample frames.
pub type Samples = u64;

/// Converts seconds to sample frames at `sample_rate`, rounding to nearest.
pub fn seconds_to_samples(seconds: f64, sample_rate: u32) -> Samples {
    (seconds.max(0.0) * f64::from(sample_rate)).round() as Samples
}

/// Converts sample frames to seconds at `sample_rate`.
pub fn samples_to_seconds(samples: Samples, sample_rate: u32) -> f64 {
    samples as f64 / f64::from(sample_rate)
}
