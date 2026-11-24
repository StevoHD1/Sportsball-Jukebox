use std::fmt;

/// Fully decoded audio, held in memory so playback never touches the disk.
///
/// Samples are interleaved `f32` frames at a fixed sample rate. Buffers are
/// shared between clips with `Arc`, so the same sound can appear on a
/// timeline many times without copying.
#[derive(Clone, PartialEq)]
pub struct AudioBuffer {
    sample_rate: u32,
    channels: u16,
    data: Vec<f32>,
}

/// Why an [`AudioBuffer`] could not be created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BufferError {
    /// The buffer must have at least one channel.
    NoChannels,
    /// The sample rate must be non-zero.
    InvalidSampleRate,
    /// The sample count is not a whole number of frames.
    PartialFrame { samples: usize, channels: u16 },
}

impl fmt::Display for BufferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoChannels => write!(f, "audio buffer must have at least one channel"),
            Self::InvalidSampleRate => write!(f, "audio buffer sample rate must be non-zero"),
            Self::PartialFrame { samples, channels } => write!(
                f,
                "{samples} samples is not a whole number of {channels}-channel frames"
            ),
        }
    }
}

impl std::error::Error for BufferError {}

impl AudioBuffer {
    /// Creates a buffer from interleaved samples.
    pub fn from_interleaved(
        sample_rate: u32,
        channels: u16,
        data: Vec<f32>,
    ) -> Result<Self, BufferError> {
        if channels == 0 {
            return Err(BufferError::NoChannels);
        }
        if sample_rate == 0 {
            return Err(BufferError::InvalidSampleRate);
        }
        if data.len() % usize::from(channels) != 0 {
            return Err(BufferError::PartialFrame {
                samples: data.len(),
                channels,
            });
        }
        Ok(Self {
            sample_rate,
            channels,
            data,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Number of frames (samples per channel).
    pub fn frames(&self) -> u64 {
        (self.data.len() / usize::from(self.channels)) as u64
    }

    /// Interleaved sample data.
    pub fn data(&self) -> &[f32] {
        &self.data
    }
}

impl fmt::Debug for AudioBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AudioBuffer")
            .field("sample_rate", &self.sample_rate)
            .field("channels", &self.channels)
            .field("frames", &self.frames())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_partial_frames() {
        assert_eq!(
            AudioBuffer::from_interleaved(48_000, 2, vec![0.0; 3]),
            Err(BufferError::PartialFrame {
                samples: 3,
                channels: 2
            })
        );
    }

    #[test]
    fn counts_frames() {
        let b = AudioBuffer::from_interleaved(48_000, 2, vec![0.0; 10]).unwrap();
        assert_eq!(b.frames(), 5);
    }
}
