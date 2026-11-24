use sbj_media::{MediaError, decode_bytes, resample};

/// Builds a 16-bit PCM WAV file in memory.
fn wav(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&channels.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
    b.extend_from_slice(&(channels * 2).to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        b.extend_from_slice(&s.to_le_bytes());
    }
    b
}

#[test]
fn decodes_stereo_wav_exactly() {
    let samples: Vec<i16> = (0..2000).map(|i| (i * 7 % 30_000) as i16).collect();
    let buf = decode_bytes(wav(44_100, 2, &samples), Some("wav")).unwrap();
    assert_eq!(buf.sample_rate(), 44_100);
    assert_eq!(buf.channels(), 2);
    assert_eq!(buf.frames(), 1000);
    for (decoded, original) in buf.data().iter().zip(&samples) {
        assert!((decoded - f32::from(*original) / 32768.0).abs() < 1e-6);
    }
}

#[test]
fn rejects_garbage() {
    let err = decode_bytes(b"definitely not audio".repeat(100), None).unwrap_err();
    assert!(matches!(err, MediaError::Unsupported(_)), "{err:?}");
}

#[test]
fn resamples_44k1_to_48k_preserving_length_and_pitch() {
    // One second of a 1 kHz sine at 44.1 kHz.
    let rate_in = 44_100;
    let samples: Vec<i16> = (0..rate_in)
        .map(|i| {
            let t = i as f32 / rate_in as f32;
            ((t * 1000.0 * std::f32::consts::TAU).sin() * 16_000.0) as i16
        })
        .collect();
    let buf = decode_bytes(wav(rate_in, 1, &samples), Some("wav")).unwrap();
    let out = resample(&buf, 48_000).unwrap();
    assert_eq!(out.sample_rate(), 48_000);
    assert!(
        (out.frames() as i64 - 48_000).abs() <= 1,
        "{}",
        out.frames()
    );

    // Count zero crossings in the steady middle: 1 kHz => ~2 per ms.
    let mid = &out.data()[10_000..38_000];
    let crossings = mid
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count();
    let expected = 2.0 * 1000.0 * (mid.len() as f32 / 48_000.0);
    assert!(
        (crossings as f32 - expected).abs() < 4.0,
        "{crossings} vs {expected}"
    );
    // Amplitude preserved.
    let peak = mid.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!((peak - 16_000.0 / 32768.0).abs() < 0.01, "{peak}");
}

#[test]
fn resample_to_same_rate_is_identity() {
    let buf = decode_bytes(wav(48_000, 1, &[1, 2, 3]), Some("wav")).unwrap();
    assert_eq!(resample(&buf, 48_000).unwrap(), buf);
}
