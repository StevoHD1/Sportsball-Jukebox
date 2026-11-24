//! A backend with no hardware: audio is pulled on demand by calling
//! [`OfflineStream::render`]. Used for tests, CI and rendering to a file.

use std::sync::Arc;

use crate::{
    AudioBackend, BackendError, DeviceId, DeviceInfo, OutputStream, RenderCallback, StreamConfig,
    StreamStatus,
};

pub const OFFLINE_DEVICE: &str = "offline";
const DEFAULT_BLOCK: u32 = 512;

/// An offline backend. The rate and channel count it reports as the
/// device's "current" configuration are configurable for tests.
#[derive(Debug, Clone, Copy)]
pub struct OfflineBackend {
    pub sample_rate: u32,
    pub channels: u16,
}

impl Default for OfflineBackend {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            channels: 2,
        }
    }
}

pub struct OfflineStream {
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

    /// Simulates the device disappearing, for testing failure handling.
    pub fn simulate_device_lost(&self) {
        self.status.set_health(crate::StreamHealth::DeviceLost);
    }
}

impl OfflineBackend {
    fn check(&self, device: &DeviceId) -> Result<(), BackendError> {
        if device.0 == OFFLINE_DEVICE {
            Ok(())
        } else {
            Err(BackendError::DeviceNotFound(device.clone()))
        }
    }
}

impl AudioBackend for OfflineBackend {
    type Stream = OfflineStream;

    fn name(&self) -> &'static str {
        "Offline"
    }

    fn output_devices(&self) -> Result<Vec<DeviceInfo>, BackendError> {
        Ok(vec![DeviceInfo {
            id: DeviceId(OFFLINE_DEVICE.into()),
            name: "Offline renderer".into(),
            output_channels: self.channels,
            sample_rate: self.sample_rate,
            is_default: true,
        }])
    }

    fn default_output_device(&self) -> Result<DeviceId, BackendError> {
        Ok(DeviceId(OFFLINE_DEVICE.into()))
    }

    fn preferred_config(&self, device: &DeviceId) -> Result<StreamConfig, BackendError> {
        self.check(device)?;
        Ok(StreamConfig {
            sample_rate: self.sample_rate,
            channels: self.channels,
            buffer_frames: None,
        })
    }

    fn open_output(
        &self,
        device: &DeviceId,
        config: StreamConfig,
        callback: RenderCallback,
    ) -> Result<OfflineStream, BackendError> {
        self.check(device)?;
        let block_frames = config.buffer_frames.unwrap_or(DEFAULT_BLOCK);
        if config.channels == 0 || block_frames == 0 || config.sample_rate == 0 {
            return Err(BackendError::UnsupportedConfig(
                "channels, buffer size and sample rate must be non-zero".into(),
            ));
        }
        let block = vec![0.0; block_frames as usize * usize::from(config.channels)];
        Ok(OfflineStream {
            config,
            callback,
            block,
            status: StreamStatus::new(),
        })
    }
}
