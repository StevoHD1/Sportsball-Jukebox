//! Audio device abstraction.
//!
//! Each platform backend (Core Audio on macOS; WASAPI/ASIO if Windows is
//! ever added) implements [`AudioBackend`]. The engine knows nothing about
//! devices: a backend just calls a [`RenderCallback`] on its real-time
//! thread, and the app wires that callback to `sbj_engine::Renderer`.
//!
//! [`offline::OfflineBackend`] drives callbacks synchronously, which is how
//! the engine is exercised in tests and CI without any audio hardware.

pub mod offline;

use std::fmt;

/// Opaque, persistent identifier for an output device (on macOS this will be
/// the Core Audio device UID, which survives reboots and re-plugging).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeviceId(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub name: String,
    pub output_channels: u16,
    pub sample_rates: Vec<u32>,
    pub is_default: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamConfig {
    pub sample_rate: u32,
    pub channels: u16,
    /// Requested frames per callback. Backends may deliver different sizes,
    /// so callbacks must handle any length.
    pub buffer_frames: u32,
}

/// Called on the device's real-time thread with an interleaved output
/// buffer to fill. Must be real-time safe.
pub type RenderCallback = Box<dyn FnMut(&mut [f32]) + Send + 'static>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendError {
    DeviceNotFound(DeviceId),
    UnsupportedConfig(String),
    /// An error reported by the operating system's audio API.
    Platform(String),
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeviceNotFound(id) => write!(f, "audio device {:?} not found", id.0),
            Self::UnsupportedConfig(why) => write!(f, "unsupported stream config: {why}"),
            Self::Platform(why) => write!(f, "audio system error: {why}"),
        }
    }
}

impl std::error::Error for BackendError {}

/// A running output stream. Dropping it stops the stream.
pub trait OutputStream: Send {
    fn config(&self) -> StreamConfig;
}

pub trait AudioBackend {
    type Stream: OutputStream;

    fn name(&self) -> &'static str;

    fn output_devices(&self) -> Result<Vec<DeviceInfo>, BackendError>;

    /// Opens `device` and starts calling `callback` for audio.
    fn open_output(
        &self,
        device: &DeviceId,
        config: StreamConfig,
        callback: RenderCallback,
    ) -> Result<Self::Stream, BackendError>;
}
