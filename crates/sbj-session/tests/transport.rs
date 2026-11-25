//! Scrubbing playing pads and previews, Fade/Stop Last, single-play banks.

mod common;

use common::*;
use sbj_session::{ControlMessage, CueDoc, DeviceRef, OutputDoc};

/// Pads 0 "A" (ramp, 1 s), 1 "B" (0.5, 1 s), 2 "C" (0.25, 1 s), all in
/// bank 1 of 2 (4 pads per bank); pad 4 "D" (0.125, 10 s) in bank 2.
fn setup() -> (
    sbj_backend::offline::OfflineBackend,
    sbj_session::Session<sbj_backend::offline::OfflineBackend>,
) {
    let backend = two_devices();
    let mut s = session(&backend);
    s.set_show_settings("T", 4, 4, vec!["One".into(), "Two".into()])
        .unwrap();
    s.add_sound("a".into(), ramp(48_000));
    s.add_sound("b".into(), constant(0.5, 48_000));
    s.add_sound("c".into(), constant(0.25, 48_000));
    s.add_sound("d".into(), constant(0.125, 480_000));
    for (slot, title, file) in [(0, "A", "a"), (1, "B", "b"), (2, "C", "c"), (4, "D", "d")] {
        s.set_cue(CueDoc::simple(slot, title, file)).unwrap();
    }
    (backend, s)
}

#[test]
fn seeking_a_playing_pad_jumps_to_that_point() {
    let (_b, mut s) = setup();
    s.seek_cue(0, 0.5).unwrap(); // not playing: nothing happens
    assert!(!s.cue_state(0).unwrap().playing);
    s.fire_cue(0).unwrap();
    pull(&mut s, 0, 100);
    s.seek_cue(0, 0.5).unwrap();
    let out = pull(&mut s, 0, 10);
    assert_eq!(out, ramp_values(24_000..24_010), "plays from 0.5 s");
    s.seek_cue(0, 99.0).unwrap(); // clamped to the end
    pull(&mut s, 0, 10);
    assert!(!s.cue_state(0).unwrap().playing);
}

#[test]
fn fade_and_stop_last_act_on_the_most_recent_pad_still_playing() {
    let (_b, mut s) = setup();
    assert_eq!(s.stop_last(false).unwrap(), None, "nothing playing");
    s.fire_cue(1).unwrap();
    s.fire_cue(2).unwrap();
    pull(&mut s, 0, 10);
    assert_eq!(s.last_playing(), Some(2));
    assert_eq!(s.stop_last(false).unwrap(), Some(2));
    pull(&mut s, 0, 10);
    assert!(!s.cue_state(2).unwrap().playing && s.cue_state(1).unwrap().playing);
    assert_eq!(s.last_playing(), Some(1), "then the one before");

    // Fade Last uses the pad's fade-out; from OSC and the remote too.
    s.handle_control(ControlMessage::Osc {
        address: "/sbj/fadelast".into(),
        args: vec![],
    });
    assert!(
        s.last_control().unwrap().contains("fade last: \"B\""),
        "{:?}",
        s.last_control()
    );
    let fading = pull(&mut s, 0, 100);
    assert!(
        fading[99] < fading[0] && fading[99] > 0.0,
        "fading, not cut"
    );
    pull(&mut s, 0, 48_000);
    assert!(!s.cue_state(1).unwrap().playing);

    s.fire_cue(2).unwrap();
    pull(&mut s, 0, 10);
    let state = s.handle_remote("GET", "/api/state").unwrap();
    assert_eq!(state["last"], "C");
    s.handle_remote("POST", "/api/stoplast").unwrap();
    pull(&mut s, 0, 10);
    assert!(!s.cue_state(2).unwrap().playing);
}

#[test]
fn single_play_banks_fade_out_the_banks_other_pads() {
    let (_b, mut s) = setup();
    // Multiplay (the default): they play together.
    s.fire_cue(1).unwrap();
    s.fire_cue(2).unwrap();
    pull(&mut s, 0, 10);
    assert!(s.cue_state(1).unwrap().playing && s.cue_state(2).unwrap().playing);
    s.stop_all(0.0).unwrap();
    pull(&mut s, 0, 10);

    s.set_bank_multiplay(0, false);
    assert!(!s.bank_multiplay(0) && s.bank_multiplay(1));
    s.fire_cue(4).unwrap(); // other bank: unaffected
    s.fire_cue(1).unwrap();
    pull(&mut s, 0, 10);
    s.fire_cue(2).unwrap();
    pull(&mut s, 0, 48_000 + 100); // past the 1 s hand-over
    assert!(
        !s.cue_state(1).unwrap().playing,
        "faded out by the hand-over"
    );
    assert!(s.cue_state(4).unwrap().playing, "other bank keeps playing");

    // Saved; removing the bank forgets it.
    let back = sbj_session::Show::from_json(&s.show().to_json()).unwrap();
    assert_eq!(back.single_play, [0]);
    s.set_bank_multiplay(0, true);
    assert!(s.show().single_play.is_empty());
    s.set_bank_multiplay(1, false);
    s.remove_cue(4).unwrap();
    s.set_show_settings("T", 4, 4, vec!["One".into()]).unwrap();
    assert!(s.show().single_play.is_empty());
}

#[test]
fn the_preview_can_be_scrubbed_and_reports_its_length() {
    let (_b, mut s) = setup();
    s.set_outputs(vec![
        OutputDoc::named("Arena"),
        OutputDoc {
            name: "Headphones".into(),
            device: Some(DeviceRef {
                id: B.into(),
                name: String::new(),
            }),
            backup: None,
        },
    ])
    .unwrap();
    s.set_preview_output(Some(1)).unwrap();
    s.preview_cue(1).unwrap();
    pull(&mut s, 1, 100);
    let state = s.preview_state().unwrap();
    assert!(
        (state.duration_seconds - 1.0).abs() < 0.01,
        "{}",
        state.duration_seconds
    );
    s.seek_preview(0.75).unwrap();
    pull(&mut s, 1, 10);
    let position = s.preview_state().unwrap().position_seconds;
    assert!(position > 0.74 && position < 0.77, "{position}");
    // After it ends, scrubbing plays it again from there.
    pull(&mut s, 1, 48_000);
    assert!(!s.preview_state().unwrap().playing);
    s.seek_preview(0.5).unwrap();
    pull(&mut s, 1, 10);
    assert!(s.preview_state().unwrap().playing);
}
