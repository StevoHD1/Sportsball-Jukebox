//! Level matching: pads and playlist songs play at the show's target
//! loudness, within the boost and peak limits.

mod common;

use common::*;
use sbj_engine::AudioBuffer;
use sbj_session::{CueDoc, PlaylistDoc, Sound};

/// A mono 1 kHz sine at `amplitude`, 2 s at 48 kHz. Mono counts as both
/// speakers, so it measures 20·log10(amplitude) LUFS (e.g. 0.1 → -20).
fn sine(amplitude: f32) -> Sound {
    let data = (0..96_000)
        .map(|i| amplitude * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 48_000.0).sin())
        .collect();
    Sound::from_buffer(AudioBuffer::from_interleaved(48_000, 1, data).unwrap(), &[]).unwrap()
}

fn peak(out: &[f32]) -> f32 {
    out.iter().fold(0.0, |m, s| m.max(s.abs()))
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.003
}

#[test]
fn pads_play_at_the_target_loudness() {
    let backend = two_devices();
    let mut s = session(&backend);
    s.add_sound("quiet".into(), sine(0.05)); // -26 LUFS
    s.add_sound("loud".into(), sine(0.5)); // -6 LUFS
    s.set_cue(CueDoc::simple(0, "Quiet", "quiet")).unwrap();
    s.set_cue(CueDoc::simple(1, "Loud", "loud")).unwrap();

    let level = |s: &mut sbj_session::Session<_>, slot| {
        s.stop_all(0.0).unwrap();
        pull(s, 0, 100);
        s.fire_cue(slot).unwrap();
        peak(&pull(s, 0, 4_800))
    };
    assert!(close(level(&mut s, 0), 0.05) && close(level(&mut s, 1), 0.5));

    // At -20 LUFS both come out at a 0.1 peak.
    s.set_level_match(Some(-20.0));
    assert!(s.is_dirty());
    assert!(close(level(&mut s, 0), 0.1), "{}", level(&mut s, 0));
    assert!(close(level(&mut s, 1), 0.1), "{}", level(&mut s, 1));

    // A pad can opt out.
    let mut loud = s.show().cue(1).unwrap().clone();
    loud.level_match = false;
    s.set_cue(loud).unwrap();
    assert!(close(level(&mut s, 1), 0.5));

    // Boost is capped at +12 dB: -26 → -14, not -6.
    s.set_level_match(Some(-6.0));
    let expected = 0.05 * 10f32.powf(12.0 / 20.0);
    assert!(close(level(&mut s, 0), expected), "{}", level(&mut s, 0));

    s.set_level_match(None);
    assert!(close(level(&mut s, 0), 0.05));
}

#[test]
fn boosts_stop_at_the_peak_ceiling() {
    let backend = two_devices();
    let mut s = session(&backend);
    // A spiky sound: quiet overall but with a peak at -3 dBFS, so it can
    // only come up 2 dB before reaching -1 dBFS.
    let mut data: Vec<f32> = (0..96_000)
        .map(|i| 0.02 * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 48_000.0).sin())
        .collect();
    data[10] = 0.707;
    let sound =
        Sound::from_buffer(AudioBuffer::from_interleaved(48_000, 1, data).unwrap(), &[]).unwrap();
    s.add_sound("spiky".into(), sound);
    s.set_cue(CueDoc::simple(0, "Spiky", "spiky")).unwrap();
    s.set_level_match(Some(-14.0));
    s.fire_cue(0).unwrap();
    let out = pull(&mut s, 0, 4_800);
    assert!(
        peak(&out) <= 10f32.powf(-1.0 / 20.0) + 1e-4,
        "{}",
        peak(&out)
    );
}

#[test]
fn playlist_songs_are_matched_one_by_one() {
    let backend = two_devices();
    let mut s = session(&backend);
    s.add_sound("a".into(), sine(0.05));
    s.add_sound("b".into(), sine(0.4));
    let mut cue = CueDoc::simple(0, "Music", "a");
    cue.clips.clear();
    cue.playlist = Some(PlaylistDoc {
        items: vec!["a".into(), "b".into()],
        shuffle: false,
        auto_advance: true,
        crossfade: 0.0,
    });
    s.set_cue(cue).unwrap();
    s.set_level_match(Some(-20.0));
    s.fire_cue(0).unwrap();
    let first = peak(&pull(&mut s, 0, 4_800));
    s.skip_cue(0).unwrap();
    pull(&mut s, 0, 100);
    let second = peak(&pull(&mut s, 0, 4_800));
    assert!(close(first, 0.1) && close(second, 0.1), "{first} {second}");
}

#[test]
fn the_setting_is_saved_with_the_show() {
    let backend = two_devices();
    let mut s = session(&backend);
    s.set_level_match(Some(-16.0));
    let json = s.show().to_json();
    assert!(json.contains("\"level_match\": -16.0"), "{json}");
    let back = sbj_session::Show::from_json(&json).unwrap();
    assert_eq!(back.level_match, Some(-16.0));
    // Out-of-range targets are clamped.
    s.set_level_match(Some(3.0));
    assert_eq!(s.show().level_match, Some(-6.0));
}
