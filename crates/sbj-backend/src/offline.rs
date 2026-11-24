//! A backend with no hardware: audio is pulled on demand by calling
//! [`OfflineStream::render`]. Used for tests, CI and rendering to a file.

use crate::{
    AudioBackend, BackendError, DeviceId, DeviceInfo, OutputStream, RenderCallback, StreamConfig,
};

pub const OFFLINE_DEVICE: &str = "offline";

#[derive(Debug, Default, Clone, Copy)]
pub struct OfflineBackend;

pub struct OfflineStream {
    config: StreamConfig,
    callback: RenderCallback,
    block: Vec<f32>,
}

impl OutputStream for OfflineStream {
    fn config(&self) -> StreamConfig {
        self.config
    }
}

impl OfflineStream {
    /// Pulls `frames` frames of audio through the callback, in blocks of
    /// `config.buffer_frames`, and returns them interleaved.
    pub fn render(&mut self, frames: usize) -> Vec<f32> {
        let ch = usize::from(self.config.channels);
        let mut out = Vec::with_capacity(frames * ch);
        let mut left = frames;
        while left > 0 {
            let n = left.min(self.config.buffer_frames as usize);
            let block = &mut self.block[..n * ch];
            (self.callback)(block);
            out.extend_from_slice(block);
            left -= n;
        }
        out
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
            output_channels: 64,
            sample_rates: vec![44_100, 48_000, 88_200, 96_000],
            is_default: true,
        }])
    }

    fn open_output(
        &self,
        device: &DeviceId,
        config: StreamConfig,
        callback: RenderCallback,
    ) -> Result<OfflineStream, BackendError> {
        if device.0 != OFFLINE_DEVICE {
            return Err(BackendError::DeviceNotFound(device.clone()));
        }
        if config.channels == 0 || config.buffer_frames == 0 || config.sample_rate == 0 {
            return Err(BackendError::UnsupportedConfig(
                "channels, buffer size and sample rate must be non-zero".into(),
            ));
        }
        let block = vec![0.0; config.buffer_frames as usize * usize::from(config.channels)];
        Ok(OfflineStream {
            config,
            callback,
            block,
        })
    }
}
