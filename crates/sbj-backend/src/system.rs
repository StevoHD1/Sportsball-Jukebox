//! The operating system's audio API.
//!
//! On macOS this is Core Audio (and on Windows, WASAPI), through the cpal
//! crate. One stream is opened per device, each with its own real-time
//! thread, which is how several interfaces run independently.
//!
//! On other platforms (Linux CI) this backend compiles but reports no
//! devices, so nothing here needs ALSA headers.

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod imp {
    use std::sync::Arc;

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{BufferSize, ErrorKind, SupportedBufferSize};

    use crate::{
        AudioBackend, BackendError, DeviceId, DeviceInfo, OutputStream, RenderCallback,
        StreamConfig, StreamHealth, StreamStatus,
    };

    pub struct SystemBackend {
        host: cpal::Host,
    }

    impl Default for SystemBackend {
        fn default() -> Self {
            Self::new()
        }
    }

    impl SystemBackend {
        pub fn new() -> Self {
            Self {
                host: cpal::default_host(),
            }
        }

        fn device(&self, id: &DeviceId) -> Result<cpal::Device, BackendError> {
            let parsed: cpal::DeviceId =
                id.0.parse()
                    .map_err(|_| BackendError::DeviceNotFound(id.clone()))?;
            self.host
                .device_by_id(&parsed)
                .ok_or_else(|| BackendError::DeviceNotFound(id.clone()))
        }
    }

    fn platform(e: impl std::fmt::Display) -> BackendError {
        BackendError::Platform(e.to_string())
    }

    pub struct SystemStream {
        stream: cpal::Stream,
        config: StreamConfig,
        status: Arc<StreamStatus>,
    }

    impl OutputStream for SystemStream {
        fn config(&self) -> StreamConfig {
            self.config
        }

        fn status(&self) -> &StreamStatus {
            &self.status
        }
    }

    impl SystemStream {
        /// The underlying cpal stream, for platform-specific tuning.
        pub fn raw(&self) -> &cpal::Stream {
            &self.stream
        }
    }

    impl AudioBackend for SystemBackend {
        type Stream = SystemStream;

        fn name(&self) -> &'static str {
            self.host.id().name()
        }

        fn output_devices(&self) -> Result<Vec<DeviceInfo>, BackendError> {
            let default_id = self.host.default_output_device().and_then(|d| d.id().ok());
            let mut devices = Vec::new();
            for device in self.host.output_devices().map_err(platform)? {
                // Skip devices that cannot report an id or a usable config
                // (e.g. mid-disconnect) rather than failing the whole list.
                let Ok(id) = device.id() else { continue };
                let Ok(config) = device.default_output_config() else {
                    continue;
                };
                let name = device
                    .description()
                    .map(|d| d.name().to_string())
                    .unwrap_or_else(|_| id.to_string());
                devices.push(DeviceInfo {
                    is_default: default_id.as_ref() == Some(&id),
                    id: DeviceId(id.to_string()),
                    name,
                    output_channels: config.channels(),
                    sample_rate: config.sample_rate(),
                });
            }
            Ok(devices)
        }

        fn default_output_device(&self) -> Result<DeviceId, BackendError> {
            let device = self
                .host
                .default_output_device()
                .ok_or(BackendError::NoDefaultDevice)?;
            Ok(DeviceId(device.id().map_err(platform)?.to_string()))
        }

        fn preferred_config(&self, id: &DeviceId) -> Result<StreamConfig, BackendError> {
            let config = self.device(id)?.default_output_config().map_err(platform)?;
            Ok(StreamConfig {
                sample_rate: config.sample_rate(),
                channels: config.channels(),
                buffer_frames: None,
            })
        }

        fn open_output(
            &self,
            id: &DeviceId,
            config: StreamConfig,
            mut callback: RenderCallback,
        ) -> Result<SystemStream, BackendError> {
            let device = self.device(id)?;

            // Honour a requested buffer size only if the device supports it.
            let buffer_size = match (config.buffer_frames, device.default_output_config()) {
                (Some(n), Ok(default)) => match default.buffer_size() {
                    SupportedBufferSize::Range { min, max } if (*min..=*max).contains(&n) => {
                        BufferSize::Fixed(n)
                    }
                    _ => BufferSize::Default,
                },
                _ => BufferSize::Default,
            };
            let cpal_config = cpal::StreamConfig {
                channels: config.channels,
                sample_rate: config.sample_rate,
                buffer_size,
            };

            let status = StreamStatus::new();
            let error_status = Arc::clone(&status);
            let stream = device
                .build_output_stream::<f32, _, _>(
                    cpal_config,
                    move |data: &mut [f32], _| callback(data),
                    move |err: cpal::Error| match err.kind() {
                        ErrorKind::Xrun => error_status.record_xrun(),
                        // Rerouted automatically; nothing to do.
                        ErrorKind::DeviceChanged | ErrorKind::RealtimeDenied => {}
                        ErrorKind::DeviceNotAvailable => {
                            error_status.set_health(StreamHealth::DeviceLost)
                        }
                        _ => error_status.set_health(StreamHealth::Failed),
                    },
                    None,
                )
                .map_err(|e| match e.kind() {
                    ErrorKind::UnsupportedConfig => BackendError::UnsupportedConfig(e.to_string()),
                    ErrorKind::DeviceNotAvailable => BackendError::DeviceNotFound(id.clone()),
                    _ => platform(e),
                })?;
            stream.play().map_err(platform)?;

            Ok(SystemStream {
                stream,
                config,
                status,
            })
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod imp {
    use crate::{
        AudioBackend, BackendError, DeviceId, DeviceInfo, OutputStream, RenderCallback,
        StreamConfig, StreamStatus,
    };

    /// Placeholder on platforms without a supported audio API.
    #[derive(Debug, Default)]
    pub struct SystemBackend;

    impl SystemBackend {
        pub fn new() -> Self {
            Self
        }
    }

    pub struct SystemStream {
        config: StreamConfig,
        status: StreamStatus,
    }

    impl OutputStream for SystemStream {
        fn config(&self) -> StreamConfig {
            self.config
        }

        fn status(&self) -> &StreamStatus {
            &self.status
        }
    }

    impl AudioBackend for SystemBackend {
        type Stream = SystemStream;

        fn name(&self) -> &'static str {
            "None"
        }

        fn output_devices(&self) -> Result<Vec<DeviceInfo>, BackendError> {
            Ok(Vec::new())
        }

        fn default_output_device(&self) -> Result<DeviceId, BackendError> {
            Err(BackendError::NoDefaultDevice)
        }

        fn preferred_config(&self, id: &DeviceId) -> Result<StreamConfig, BackendError> {
            Err(BackendError::DeviceNotFound(id.clone()))
        }

        fn open_output(
            &self,
            id: &DeviceId,
            _config: StreamConfig,
            _callback: RenderCallback,
        ) -> Result<SystemStream, BackendError> {
            Err(BackendError::DeviceNotFound(id.clone()))
        }
    }
}

pub use imp::{SystemBackend, SystemStream};
