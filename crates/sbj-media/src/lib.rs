//! Turning audio files into engine-ready [`AudioBuffer`]s.
//!
//! Everything here runs on the control side (or a background thread) and
//! may be slow: decoding a long MP3 takes a noticeable fraction of a second.
//! The result is fully decoded audio in memory, so nothing touches the disk
//! when a cue fires.
//!
//! Supported: WAV, AIFF, CAF, FLAC, MP3, AAC/M4A, ALAC, Ogg Vorbis.

pub mod analysis;
pub use analysis::{Analysis, analyze};

use std::fmt;
use std::fs::File;
use std::io::Cursor;
use std::path::Path;

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};
use sbj_engine::AudioBuffer;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;

/// Longest file we will load, to keep a stray multi-hour file from
/// exhausting memory (one hour of 48 kHz stereo is ~1.4 GB of f32).
pub const MAX_DURATION_SECONDS: u64 = 60 * 60;

#[derive(Debug)]
pub enum MediaError {
    Io(std::io::Error),
    /// The format or codec is not supported, or the file is not audio.
    Unsupported(String),
    /// The file is damaged or could not be decoded.
    Decode(String),
    /// The file decoded to no audio at all.
    Empty,
    TooLong,
    Resample(String),
}

impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "could not read file: {e}"),
            Self::Unsupported(why) => write!(f, "unsupported audio file: {why}"),
            Self::Decode(why) => write!(f, "could not decode audio: {why}"),
            Self::Empty => write!(f, "file contains no audio"),
            Self::TooLong => write!(
                f,
                "file is longer than {} minutes",
                MAX_DURATION_SECONDS / 60
            ),
            Self::Resample(why) => write!(f, "could not convert sample rate: {why}"),
        }
    }
}

impl std::error::Error for MediaError {}

impl From<std::io::Error> for MediaError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Decodes a whole file at its native sample rate.
pub fn decode_file(path: impl AsRef<Path>) -> Result<AudioBuffer, MediaError> {
    let path = path.as_ref();
    let file = File::open(path)?;
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    decode(Box::new(file), hint)
}

/// Decodes audio held in memory. `extension` (e.g. `"wav"`) helps format
/// detection but is optional.
pub fn decode_bytes(bytes: Vec<u8>, extension: Option<&str>) -> Result<AudioBuffer, MediaError> {
    let mut hint = Hint::new();
    if let Some(ext) = extension {
        hint.with_extension(ext);
    }
    decode(Box::new(Cursor::new(bytes)), hint)
}

fn decode(source: Box<dyn MediaSource>, hint: Hint) -> Result<AudioBuffer, MediaError> {
    let stream = MediaSourceStream::new(source, Default::default());
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            stream,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| MediaError::Unsupported(e.to_string()))?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| MediaError::Unsupported("no audio track".into()))?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| MediaError::Unsupported("track has no audio parameters".into()))?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(|e| MediaError::Unsupported(e.to_string()))?;

    let mut samples: Vec<f32> = Vec::new();
    let mut chunk: Vec<f32> = Vec::new();
    let mut spec: Option<(u32, u16)> = None;
    let mut limit = usize::MAX;

    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            // Some formats report end-of-stream as an unexpected EOF.
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(e) => return Err(MediaError::Decode(e.to_string())),
        };
        if packet.track_id != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            // A corrupt packet: skip it, as players do, rather than failing
            // the whole file.
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(MediaError::Decode(e.to_string())),
        };

        let rate = decoded.spec().rate();
        let channels = decoded.spec().channels().count();
        let channels = u16::try_from(channels)
            .map_err(|_| MediaError::Unsupported(format!("{channels} channels")))?;
        match spec {
            None => {
                spec = Some((rate, channels));
                limit = (MAX_DURATION_SECONDS as usize)
                    .saturating_mul(rate as usize)
                    .saturating_mul(usize::from(channels));
            }
            Some(s) if s != (rate, channels) => {
                return Err(MediaError::Decode(
                    "sample rate or channel count changed mid-file".into(),
                ));
            }
            Some(_) => {}
        }

        chunk.clear();
        decoded.copy_to_vec_interleaved(&mut chunk);
        samples.extend_from_slice(&chunk);
        if samples.len() > limit {
            return Err(MediaError::TooLong);
        }
    }

    let (rate, channels) = spec.ok_or(MediaError::Empty)?;
    if samples.is_empty() {
        return Err(MediaError::Empty);
    }
    AudioBuffer::from_interleaved(rate, channels, samples)
        .map_err(|e| MediaError::Decode(e.to_string()))
}

/// Converts `buffer` to `target_rate` with a high-quality FFT resampler.
/// Returns a copy unchanged if the rate already matches.
pub fn resample(buffer: &AudioBuffer, target_rate: u32) -> Result<AudioBuffer, MediaError> {
    if buffer.sample_rate() == target_rate {
        return Ok(buffer.clone());
    }
    if target_rate == 0 {
        return Err(MediaError::Resample("target rate must be non-zero".into()));
    }
    let channels = usize::from(buffer.channels());
    let frames = buffer.frames() as usize;
    let input = InterleavedSlice::new(buffer.data(), channels, frames)
        .map_err(|e| MediaError::Resample(e.to_string()))?;
    let mut resampler = Fft::<f32>::new(
        buffer.sample_rate() as usize,
        target_rate as usize,
        1024,
        channels,
        FixedSync::Input,
    )
    .map_err(|e| MediaError::Resample(e.to_string()))?;
    let output = resampler
        .process_all(&input, frames, None)
        .map_err(|e| MediaError::Resample(e.to_string()))?;
    AudioBuffer::from_interleaved(target_rate, buffer.channels(), output.take_data())
        .map_err(|e| MediaError::Resample(e.to_string()))
}

/// Decodes a file and converts it to `target_rate`.
pub fn load_for_rate(path: impl AsRef<Path>, target_rate: u32) -> Result<AudioBuffer, MediaError> {
    resample(&decode_file(path)?, target_rate)
}
