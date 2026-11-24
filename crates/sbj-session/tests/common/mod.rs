#![allow(dead_code)]

use std::path::Path;

use sbj_backend::offline::{OfflineBackend, OfflineDevice};
use sbj_engine::AudioBuffer;
use sbj_session::{Session, Sound};

pub const A: &str = "dev-a";
pub const B: &str = "dev-b";

/// Device A (48 kHz stereo, system default) and device B (44.1 kHz mono).
pub fn two_devices() -> OfflineBackend {
    let mut a = OfflineDevice::new(A, 48_000, 2);
    a.is_default = true;
    let b = OfflineDevice::new(B, 44_100, 1);
    OfflineBackend::with_devices(vec![a, b])
}

pub fn session(backend: &OfflineBackend) -> Session<OfflineBackend> {
    let mut s = Session::new(backend.clone());
    s.open_outputs();
    s
}

/// A mono 48 kHz sound of constant `value`.
pub fn constant(value: f32, frames: usize) -> Sound {
    Sound::from_buffer(
        AudioBuffer::from_interleaved(48_000, 1, vec![value; frames]).unwrap(),
        &[],
    )
    .unwrap()
}

/// A mono 48 kHz sound whose sample i is (i + 1) / 1e6, so output reveals
/// which source frame played. (Small values keep it under 1.0.)
pub fn ramp(frames: usize) -> Sound {
    Sound::from_buffer(
        AudioBuffer::from_interleaved(
            48_000,
            1,
            (0..frames).map(|i| (i + 1) as f32 / 1e6).collect(),
        )
        .unwrap(),
        &[],
    )
    .unwrap()
}

pub fn ramp_values(range: std::ops::Range<usize>) -> Vec<f32> {
    range.map(|i| (i + 1) as f32 / 1e6).collect()
}

/// Pulls frames from output `index` and returns channel 0.
pub fn pull(s: &mut Session<OfflineBackend>, index: usize, frames: usize) -> Vec<f32> {
    let stream = s.output_stream_mut(index).expect("output open");
    let ch = usize::from(sbj_backend::OutputStream::config(stream).channels);
    stream.render(frames).chunks(ch).map(|f| f[0]).collect()
}

/// Writes a 16-bit mono WAV of constant `value`.
pub fn write_wav(path: &Path, rate: u32, frames: usize, value: i16) {
    let data_len = (frames * 2) as u32;
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for _ in 0..frames {
        b.extend_from_slice(&value.to_le_bytes());
    }
    std::fs::write(path, b).unwrap();
}
