//! Loudness, peak and silence analysis of decoded audio (control side).
//!
//! Loudness is integrated programme loudness per ITU-R BS.1770-4 / EBU
//! R128: K-weighting (a high shelf and a high pass), mean square over
//! 400 ms blocks with 75 % overlap, an absolute gate at -70 LUFS and a
//! relative gate 10 LU below the ungated level. Sounds shorter than one
//! block are measured as a single block, so short stings still get a
//! value.
//!
//! A **mono** file is measured as if played on both speakers of a stereo
//! pair (counted twice, +3 dB), because that is how the engine plays it:
//! a mono clip feeds every output channel.

use sbj_engine::AudioBuffer;

/// Samples quieter than this (about -60 dBFS) count as silence.
pub const SILENCE_THRESHOLD: f32 = 0.001;
/// Kept before the first sound when trimming, so an attack is not cut.
pub const TRIM_PRE_ROLL_SECONDS: f64 = 0.005;

const ABSOLUTE_GATE_LUFS: f64 = -70.0;
const RELATIVE_GATE_LU: f64 = -10.0;

/// What [`analyze`] measured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Analysis {
    /// Integrated loudness in LUFS; `None` for silence.
    pub loudness_lufs: Option<f64>,
    /// Largest absolute sample value (1.0 = 0 dBFS).
    pub peak: f32,
    /// Silence before the first sound, minus a short pre-roll (seconds).
    pub lead_silence: f64,
    /// Silence after the last sound (seconds).
    pub tail_silence: f64,
}

impl Analysis {
    /// Peak in dBFS (`-inf` for silence).
    pub fn peak_dbfs(&self) -> f64 {
        20.0 * f64::from(self.peak).log10()
    }

    /// Gain (linear) that brings the sound to `target_lufs`, boosting by at
    /// most `max_boost_db` and never pushing the peak above
    /// `ceiling_dbfs`. Silence and unmeasurable sounds get 1.0.
    pub fn gain_to(&self, target_lufs: f64, max_boost_db: f64, ceiling_dbfs: f64) -> f32 {
        let Some(loudness) = self.loudness_lufs else {
            return 1.0;
        };
        let mut db = (target_lufs - loudness).min(max_boost_db);
        if self.peak > 0.0 {
            db = db.min(ceiling_dbfs - self.peak_dbfs());
        }
        10f64.powf(db / 20.0) as f32
    }
}

/// One biquad section (direct form I, f64 state).
#[derive(Clone, Copy)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    x: [f64; 2],
    y: [f64; 2],
}

impl Biquad {
    fn new(b: [f64; 3], a: [f64; 2]) -> Self {
        Self {
            b,
            a,
            x: [0.0; 2],
            y: [0.0; 2],
        }
    }

    #[inline]
    fn run(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.b[1] * self.x[0] + self.b[2] * self.x[1]
            - self.a[0] * self.y[0]
            - self.a[1] * self.y[1];
        self.x = [x, self.x[0]];
        self.y = [y, self.y[0]];
        y
    }
}

/// The two K-weighting stages for `rate`, from the BS.1770 analog
/// prototypes (as in libebur128), so any sample rate works.
fn k_weighting(rate: u32) -> (Biquad, Biquad) {
    use std::f64::consts::PI;
    let fs = f64::from(rate);

    // Stage 1: high shelf (+4 dB above ~1.5 kHz, head diffraction).
    let f0 = 1_681.974_450_955_533;
    let g = 3.999_843_853_973_347;
    let q = 0.707_175_236_955_419_6;
    let k = (PI * f0 / fs).tan();
    let vh = 10f64.powf(g / 20.0);
    let vb = vh.powf(0.499_666_774_154_541_6);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Biquad::new(
        [
            (vh + vb * k / q + k * k) / a0,
            2.0 * (k * k - vh) / a0,
            (vh - vb * k / q + k * k) / a0,
        ],
        [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
    );

    // Stage 2: high pass (RLB weighting, ~38 Hz).
    let f0 = 38.135_470_876_024_44;
    let q = 0.500_327_037_323_877_3;
    let k = (PI * f0 / fs).tan();
    let a0 = 1.0 + k / q + k * k;
    let high_pass = Biquad::new(
        [1.0, -2.0, 1.0],
        [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
    );
    (shelf, high_pass)
}

/// BS.1770 channel weights: L, R, C = 1, LFE (4th of 6+) = 0, surrounds
/// 1.41. Mono counts twice (see the module docs).
fn channel_weight(channel: usize, channels: usize) -> f64 {
    match (channels, channel) {
        (1, _) => 2.0,
        (c, 3) if c >= 6 => 0.0,
        (c, 4 | 5) if c >= 6 => 1.41,
        _ => 1.0,
    }
}

/// Measures loudness, peak and leading/trailing silence.
pub fn analyze(buffer: &AudioBuffer) -> Analysis {
    let rate = buffer.sample_rate();
    let channels = usize::from(buffer.channels().max(1));
    let data = buffer.data();
    let frames = data.len() / channels;

    // Peak and silence.
    let mut peak = 0f32;
    let mut first = None;
    let mut last = None;
    for (i, frame) in data.chunks_exact(channels).enumerate() {
        let loudest = frame.iter().fold(0f32, |m, s| m.max(s.abs()));
        peak = peak.max(loudest);
        if loudest > SILENCE_THRESHOLD {
            first.get_or_insert(i);
            last = Some(i);
        }
    }
    let seconds = |f: usize| f as f64 / f64::from(rate.max(1));
    let (lead_silence, tail_silence) = match (first, last) {
        (Some(a), Some(b)) => (
            (seconds(a) - TRIM_PRE_ROLL_SECONDS).max(0.0),
            seconds(frames - 1 - b),
        ),
        _ => (seconds(frames), 0.0),
    };

    Analysis {
        loudness_lufs: loudness(data, channels, rate),
        peak,
        lead_silence,
        tail_silence,
    }
}

/// Integrated loudness (LUFS) of interleaved `data`, or `None` if it is
/// silent (everything below the absolute gate) or empty.
fn loudness(data: &[f32], channels: usize, rate: u32) -> Option<f64> {
    let frames = data.len() / channels;
    if frames == 0 || rate == 0 {
        return None;
    }
    // Weighted K-filtered energy summed per 100 ms segment; a 400 ms block
    // is four consecutive segments (75 % overlap).
    let segment = (rate as usize / 10).max(1);
    let mut segments = vec![0f64; frames.div_ceil(segment)];
    for c in 0..channels {
        let weight = channel_weight(c, channels);
        if weight == 0.0 {
            continue;
        }
        let (mut shelf, mut high_pass) = k_weighting(rate);
        for (i, frame) in data.chunks_exact(channels).enumerate() {
            let y = high_pass.run(shelf.run(f64::from(frame[c])));
            segments[i / segment] += weight * y * y;
        }
    }

    let full = frames / segment; // complete segments
    let blocks: Vec<f64> = if full >= 4 {
        (0..=full - 4)
            .map(|i| segments[i..i + 4].iter().sum::<f64>() / (4 * segment) as f64)
            .collect()
    } else {
        // Shorter than one block: measure the whole sound as one.
        vec![segments.iter().sum::<f64>() / frames as f64]
    };

    let lufs = |power: f64| -0.691 + 10.0 * power.log10();
    let gated: Vec<f64> = blocks
        .into_iter()
        .filter(|&p| p > 0.0 && lufs(p) > ABSOLUTE_GATE_LUFS)
        .collect();
    if gated.is_empty() {
        return None;
    }
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    let relative = lufs(mean(&gated)) + RELATIVE_GATE_LU;
    let kept: Vec<f64> = gated.into_iter().filter(|&p| lufs(p) > relative).collect();
    Some(lufs(mean(&kept)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, seconds: f64, amplitude: f32, channels: &[bool]) -> AudioBuffer {
        let frames = (seconds * f64::from(rate)) as usize;
        let mut data = Vec::with_capacity(frames * channels.len());
        for i in 0..frames {
            let s = amplitude
                * (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / f64::from(rate)).sin() as f32;
            for &on in channels {
                data.push(if on { s } else { 0.0 });
            }
        }
        AudioBuffer::from_interleaved(rate, channels.len() as u16, data).unwrap()
    }

    fn lufs(buffer: &AudioBuffer) -> f64 {
        analyze(buffer).loudness_lufs.unwrap()
    }

    #[test]
    fn reference_tone_reads_as_specified() {
        // BS.1770: a 0 dBFS 1 kHz sine in one channel reads -3.01 LKFS.
        for rate in [44_100, 48_000, 96_000] {
            let l = lufs(&sine(rate, 5.0, 1.0, &[true, false]));
            assert!((l + 3.01).abs() < 0.05, "{rate} Hz: {l}");
        }
        // Both channels: +3 dB. 20 dB quieter: -20.
        let l = lufs(&sine(48_000, 5.0, 0.1, &[true, true]));
        assert!((l + 20.0).abs() < 0.05, "{l}");
        // Mono counts as both speakers.
        let l = lufs(&sine(48_000, 5.0, 0.1, &[true]));
        assert!((l + 20.0).abs() < 0.05, "{l}");
    }

    #[test]
    fn silence_is_gated_out() {
        // 3 s of tone then 7 s of silence: the silent blocks are gated
        // out. (The 3 blocks straddling the end are partly tone and still
        // count, as R128 specifies: about -0.2 dB.)
        let tone = sine(48_000, 3.0, 0.1, &[true, true]);
        let mut data = tone.data().to_vec();
        data.extend(std::iter::repeat_n(0.0, 48_000 * 7 * 2));
        let padded = AudioBuffer::from_interleaved(48_000, 2, data).unwrap();
        assert!((lufs(&padded) - lufs(&tone)).abs() < 0.3);

        let silent = AudioBuffer::from_interleaved(48_000, 2, vec![0.0; 96_000]).unwrap();
        let a = analyze(&silent);
        assert_eq!(a.loudness_lufs, None);
        assert_eq!(a.gain_to(-16.0, 12.0, -1.0), 1.0);
    }

    #[test]
    fn short_stings_are_measured() {
        let l = lufs(&sine(48_000, 0.2, 0.1, &[true, true]));
        assert!((l + 20.0).abs() < 0.3, "{l}");
    }

    #[test]
    fn silence_at_the_ends_is_found() {
        let rate = 48_000;
        let tone = sine(rate, 1.0, 0.5, &[true, true]);
        let mut data = vec![0.0; rate as usize / 2 * 2]; // 0.5 s lead
        data.extend_from_slice(tone.data());
        data.extend(std::iter::repeat_n(0.0, rate as usize / 4 * 2)); // 0.25 s tail
        let a = analyze(&AudioBuffer::from_interleaved(rate, 2, data).unwrap());
        // The sine's first sample is 0, so sound starts one frame in.
        assert!((a.lead_silence - (0.5 - TRIM_PRE_ROLL_SECONDS)).abs() < 0.001);
        assert!((a.tail_silence - 0.25).abs() < 0.001, "{}", a.tail_silence);
        assert!((a.peak - 0.5).abs() < 1e-3);
    }

    #[test]
    fn gain_respects_boost_and_peak_limits() {
        let quiet = Analysis {
            loudness_lufs: Some(-30.0),
            peak: 0.1, // -20 dBFS
            lead_silence: 0.0,
            tail_silence: 0.0,
        };
        let db = |g: f32| 20.0 * f64::from(g).log10();
        // Wants +14 dB; the boost cap allows +12; the peak allows +19.
        assert!((db(quiet.gain_to(-16.0, 12.0, -1.0)) - 12.0).abs() < 1e-3);
        let hot_peak = Analysis {
            peak: 0.5, // -6 dBFS: only about +5 dB to -1 dBFS
            ..quiet
        };
        let allowed = -1.0 - 20.0 * 0.5f64.log10(); // 5.02 dB
        assert!((db(hot_peak.gain_to(-16.0, 12.0, -1.0)) - allowed).abs() < 1e-3);
        let loud = Analysis {
            loudness_lufs: Some(-8.0),
            peak: 1.0,
            ..quiet
        };
        assert!((db(loud.gain_to(-16.0, 12.0, -1.0)) + 8.0).abs() < 1e-3);
    }
}
