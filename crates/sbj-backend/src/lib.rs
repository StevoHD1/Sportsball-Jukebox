//! Audio device abstraction.
//!
//! Each backend implements [`AudioBackend`]. The engine knows nothing about
//! devices: a backend calls a [`RenderCallback`] on its real-time thread,
//! and the app connects that callback to `sbj_engine::Renderer::process`.
//!
//! * [`system::SystemBackend`]: the operating system's audio API via cpal
//!   (Core Audio on macOS; WASAPI on Windows). On other platforms it reports
//!   no devices.
//! * [`offline::OfflineBackend`]: no hardware; audio is pulled on demand.
//!   Used by tests and CI.

pub mod offline;
pub mod system;

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

/// Persistent identifier for an output device. On macOS this wraps the Core
/// Audio device UID, which survives reboots and re-plugging, so show files
/// can remember routing.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeviceId(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub name: String,
    pub output_channels: u16,
    /// The rate the device is currently running at. We use the device's
    /// own rate rather than changing it, since other apps share it.
    pub sample_rate: u32,
    pub is_default: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamConfig {
    pub sample_rate: u32,
    pub channels: u16,
    /// Requested frames per callback, or `None` for the device default.
    /// Backends may still deliver other sizes, so callbacks must handle any
    /// length.
    pub buffer_frames: Option<u32>,
}

/// Called on the device's real-time thread with an interleaved output
/// buffer to fill. Must be real-time safe.
pub type RenderCallback = Box<dyn FnMut(&mut [f32]) + Send + 'static>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendError {
    DeviceNotFound(DeviceId),
    NoDefaultDevice,
    UnsupportedConfig(String),
    /// An error reported by the operating system's audio API.
    Platform(String),
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeviceNotFound(id) => write!(f, "audio device {:?} not found", id.0),
            Self::NoDefaultDevice => write!(f, "no default audio output device"),
            Self::UnsupportedConfig(why) => write!(f, "unsupported stream config: {why}"),
            Self::Platform(why) => write!(f, "audio system error: {why}"),
        }
    }
}

impl std::error::Error for BackendError {}

/// How a running stream is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum StreamHealth {
    Running = 0,
    /// The device was unplugged or otherwise disappeared. The stream must be
    /// reopened on another device.
    DeviceLost = 1,
    /// The stream failed for another reason and must be reopened.
    Failed = 2,
}

/// Health flags written from the backend's error callback and read from the
/// control thread. Lock-free.
#[derive(Debug, Default)]
pub struct StreamStatus {
    health: AtomicU8,
    xruns: AtomicU64,
}

impl StreamStatus {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn health(&self) -> StreamHealth {
        match self.health.load(Ordering::Relaxed) {
            0 => StreamHealth::Running,
            1 => StreamHealth::DeviceLost,
            _ => StreamHealth::Failed,
        }
    }

    pub fn set_health(&self, health: StreamHealth) {
        // Never downgrade a failure back to running.
        self.health.fetch_max(health as u8, Ordering::Relaxed);
    }

    /// Buffer underruns/overruns reported by the device (audible glitches).
    pub fn xruns(&self) -> u64 {
        self.xruns.load(Ordering::Relaxed)
    }

    pub fn record_xrun(&self) {
        self.xruns.fetch_add(1, Ordering::Relaxed);
    }
}

/// A running output stream. Dropping it stops the stream.
pub trait OutputStream: Send {
    fn config(&self) -> StreamConfig;
    fn status(&self) -> &StreamStatus;
}

pub trait AudioBackend {
    type Stream: OutputStream;

    fn name(&self) -> &'static str;

    fn output_devices(&self) -> Result<Vec<DeviceInfo>, BackendError>;

    fn default_output_device(&self) -> Result<DeviceId, BackendError>;

    /// The configuration a stream on `device` should use: the device's
    /// current sample rate and channel count.
    fn preferred_config(&self, device: &DeviceId) -> Result<StreamConfig, BackendError>;

    /// Opens `device` and starts calling `callback` for audio.
    fn open_output(
        &self,
        device: &DeviceId,
        config: StreamConfig,
        callback: RenderCallback,
    ) -> Result<Self::Stream, BackendError>;
}
