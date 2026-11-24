//! A backend with no hardware: audio is pulled on demand by calling
//! [`OfflineStream::render`]. Used for tests, CI and rendering to a file.
//!
//! It can simulate several devices, and unplugging and re-plugging them,
//! so failover logic can be tested without audio hardware.

use std::sync::{Arc, Mutex, MutexGuard, Weak};

use crate::{
    AudioBackend, BackendError, DeviceId, DeviceInfo, OutputStream, RenderCallback, StreamConfig,
    StreamHealth, StreamStatus,
};

pub const OFFLINE_DEVICE: &str = "offline";
const DEFAULT_BLOCK: u32 = 512;

/// A simulated device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfflineDevice {
    pub id: String,
    pub name: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub is_default: bool,
}

impl OfflineDevice {
    pub fn new(id: &str, sample_rate: u32, channels: u16) -> Self {
        Self {
            id: id.into(),
            name: format!("Offline {id}"),
            sample_rate,
            channels,
            is_default: false,
        }
    }
}

#[derive(Debug, Default)]
struct State {
    /// Devices currently "plugged in".
    present: Vec<OfflineDevice>,
    /// Statuses of opened streams, by device id, so unplugging can flag
    /// them as lost.
    streams: Vec<(String, Weak<StreamStatus>)>,
}

/// An offline backend. Clones share the same simulated devices, so a test
/// can keep a clone to unplug devices while a session owns another.
#[derive(Debug, Clone)]
pub struct OfflineBackend {
    state: Arc<Mutex<State>>,
}

impl Default for OfflineBackend {
    /// One default 48 kHz stereo device with id [`OFFLINE_DEVICE`].
    fn default() -> Self {
        let mut device = OfflineDevice::new(OFFLINE_DEVICE, 48_000, 2);
        device.name = "Offline renderer".into();
        device.is_default = true;
        Self::with_devices(vec![device])
    }
}

impl OfflineBackend {
    pub fn with_devices(devices: Vec<OfflineDevice>) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                present: devices,
                streams: Vec::new(),
            })),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Simulates unplugging a device: it disappears from the device list
    /// and any open stream on it reports [`StreamHealth::DeviceLost`].
    pub fn unplug(&self, id: &str) {
        let mut state = self.lock();
        state.present.retain(|d| d.id != id);
        for (device, status) in &state.streams {
            if device == id
                && let Some(status) = status.upgrade()
            {
                status.set_health(StreamHealth::DeviceLost);
            }
        }
    }

    /// Simulates plugging a device (back) in.
    pub fn plug(&self, device: OfflineDevice) {
        let mut state = self.lock();
        state.present.retain(|d| d.id != device.id);
        state.present.push(device);
    }

    fn find(&self, id: &DeviceId) -> Result<OfflineDevice, BackendError> {
        self.lock()
            .present
            .iter()
            .find(|d| d.id == id.0)
            .cloned()
            .ok_or_else(|| BackendError::DeviceNotFound(id.clone()))
    }
}

pub struct OfflineStream {
    device: String,
    config: StreamConfig,
    callback: RenderCallback,
    block: Vec<f32>,
    status: Arc<StreamStatus>,
}

impl OutputStream for OfflineStream {
    fn config(&self) -> StreamConfig {
        self.config
    }

    fn status(&self) -> &StreamStatus {
        &self.status
    }
}

impl OfflineStream {
    /// Pulls `frames` frames of audio through the callback, in blocks of
    /// the configured buffer size, and returns them interleaved.
    pub fn render(&mut self, frames: usize) -> Vec<f32> {
        let ch = usize::from(self.config.channels);
        let block_frames = self.block.len() / ch;
        let mut out = Vec::with_capacity(frames * ch);
        let mut left = frames;
        while left > 0 {
            let n = left.min(block_frames);
            let block = &mut self.block[..n * ch];
            (self.callback)(block);
            out.extend_from_slice(block);
            left -= n;
        }
        out
    }

    /// The id of the device this stream plays to.
    pub fn device(&self) -> &str {
        &self.device
    }

    /// Simulates the device disappearing, for testing failure handling.
    pub fn simulate_device_lost(&self) {
        self.status.set_health(StreamHealth::DeviceLost);
    }
}

impl AudioBackend for OfflineBackend {
    type Stream = OfflineStream;

    fn name(&self) -> &'static str {
        "Offline"
    }

    fn output_devices(&self) -> Result<Vec<DeviceInfo>, BackendError> {
        Ok(self
            .lock()
            .present
            .iter()
            .map(|d| DeviceInfo {
                id: DeviceId(d.id.clone()),
                name: d.name.clone(),
                output_channels: d.channels,
                sample_rate: d.sample_rate,
                is_default: d.is_default,
            })
            .collect())
    }

    fn default_output_device(&self) -> Result<DeviceId, BackendError> {
        self.lock()
            .present
            .iter()
            .find(|d| d.is_default)
            .map(|d| DeviceId(d.id.clone()))
            .ok_or(BackendError::NoDefaultDevice)
    }

    fn preferred_config(&self, id: &DeviceId) -> Result<StreamConfig, BackendError> {
        let device = self.find(id)?;
        Ok(StreamConfig {
            sample_rate: device.sample_rate,
            channels: device.channels,
            buffer_frames: None,
        })
    }

    fn open_output(
        &self,
        id: &DeviceId,
        config: StreamConfig,
        callback: RenderCallback,
    ) -> Result<OfflineStream, BackendError> {
        self.find(id)?;
        let block_frames = config.buffer_frames.unwrap_or(DEFAULT_BLOCK);
        if config.channels == 0 || block_frames == 0 || config.sample_rate == 0 {
            return Err(BackendError::UnsupportedConfig(
                "channels, buffer size and sample rate must be non-zero".into(),
            ));
        }
        let status = StreamStatus::new();
        let mut state = self.lock();
        state.streams.retain(|(_, s)| s.strong_count() > 0);
        state.streams.push((id.0.clone(), Arc::downgrade(&status)));
        Ok(OfflineStream {
            device: id.0.clone(),
            config,
            callback,
            block: vec![0.0; block_frames as usize * usize::from(config.channels)],
            status,
        })
    }
}
