//! Pad actions and play modes.

mod common;

use common::*;
use sbj_session::{CheckLevel, CueDoc, FireAction, FireActionKind, PlayMode};

/// 2 banks of 4. Pads 0 "Music" (0.5), 1 "Horn" (0.25), 2 "Crowd"
/// (0.125), 4 "Other bank" (0.0625); all 10 s long.
fn setup() -> (
    sbj_backend::offline::OfflineBackend,
    sbj_session::Session<sbj_backend::offline::OfflineBackend>,
) {
    let backend = two_devices();
    let mut s = session(&backend);
    s.set_show_settings("T", 4, 4, vec!["One".into(), "Two".into()])
        .unwrap();
    for (slot, title, value) in [
        (0, "Music", 0.5),
        (1, "Horn", 0.25),
        (2, "Crowd", 0.125),
        (4, "Other bank", 0.0625),
    ] {
        let file = title.to_lowercase();
        s.add_sound(file.clone(), constant(value, 480_000));
        s.set_cue(CueDoc::simple(slot, title, &file)).unwrap();
    }
    (backend, s)
}

fn with_actions(
    s: &mut sbj_session::Session<sbj_backend::offline::OfflineBackend>,
    slot: u32,
    actions: Vec<FireAction>,
) {
    let mut cue = s.show().cue(slot).unwrap().clone();
    cue.on_fire = actions;
    s.set_cue(cue).unwrap();
}

fn playing(s: &sbj_session::Session<sbj_backend::offline::OfflineBackend>, slot: u32) -> bool {
    s.cue_state(slot).unwrap().playing
}

#[test]
fn play_over_and_solo_override_the_banks_multiplay() {
    let (_b, mut s) = setup();
    // Single-play bank: a "play over" horn doesn't stop the music, and
    // the music firing again doesn't stop the horn.
    s.set_bank_multiplay(0, false);
    let mut horn = s.show().cue(1).unwrap().clone();
    horn.play_mode = PlayMode::Over;
    s.set_cue(horn).unwrap();
    s.fire_cue(0).unwrap();
    pull(&mut s, 0, 10);
    s.fire_cue(1).unwrap();
    pull(&mut s, 0, 48_000 + 100);
    assert!(
        playing(&s, 0) && playing(&s, 1),
        "horn plays over the music"
    );
    s.fire_cue(2).unwrap(); // a normal pad in a single-play bank
    pull(&mut s, 0, 48_000 + 100);
    assert!(!playing(&s, 0), "music handed over");
    assert!(playing(&s, 1), "the horn plays over everything");

    // Multiplay bank: a solo pad still fades the bank's others out.
    s.stop_all(0.0).unwrap();
    s.set_bank_multiplay(0, true);
    let mut crowd = s.show().cue(2).unwrap().clone();
    crowd.play_mode = PlayMode::Solo;
    s.set_cue(crowd).unwrap();
    s.fire_cue(0).unwrap();
    s.fire_cue(4).unwrap();
    pull(&mut s, 0, 10);
    s.fire_cue(2).unwrap();
    pull(&mut s, 0, 48_000 + 100);
    assert!(!playing(&s, 0), "solo faded the music");
    assert!(playing(&s, 4), "other bank untouched");
    assert!(playing(&s, 2));
}

#[test]
fn stop_others_fire_and_stop_pad_and_switch_bank() {
    let (_b, mut s) = setup();
    with_actions(
        &mut s,
        1,
        vec![
            FireAction {
                seconds: Some(0.0),
                ..FireAction::new(FireActionKind::StopOthers)
            },
            FireAction {
                pad: Some(2),
                ..FireAction::new(FireActionKind::FirePad)
            },
            FireAction {
                bank: Some(1),
                ..FireAction::new(FireActionKind::Bank)
            },
        ],
    );
    s.fire_cue(0).unwrap();
    s.fire_cue(4).unwrap();
    pull(&mut s, 0, 10);
    s.fire_cue(1).unwrap();
    pull(&mut s, 0, 10);
    assert!(
        !playing(&s, 0) && !playing(&s, 4),
        "everything else stopped"
    );
    assert!(
        playing(&s, 1) && playing(&s, 2),
        "horn plus the pad it fires"
    );
    assert_eq!(s.current_bank(), 1);

    // "This bank only" leaves other banks alone; Stop Pad stops one pad.
    s.stop_all(0.0).unwrap();
    pull(&mut s, 0, 10);
    with_actions(
        &mut s,
        1,
        vec![
            FireAction {
                seconds: Some(0.0),
                this_bank: true,
                ..FireAction::new(FireActionKind::StopOthers)
            },
            FireAction {
                pad: Some(4),
                seconds: Some(0.0),
                ..FireAction::new(FireActionKind::StopPad)
            },
        ],
    );
    s.fire_cue(0).unwrap();
    s.fire_cue(2).unwrap();
    s.fire_cue(4).unwrap();
    pull(&mut s, 0, 10);
    s.fire_cue(1).unwrap();
    pull(&mut s, 0, 10);
    assert!(!playing(&s, 0) && !playing(&s, 2));
    assert!(!playing(&s, 4), "stopped by Stop Pad");
}

#[test]
fn delayed_fire_and_logging() {
    let dir = tempfile::tempdir().unwrap();
    let (_b, mut s) = setup();
    s.set_log_dir(Some(dir.path().to_path_buf()));
    with_actions(
        &mut s,
        1,
        vec![FireAction {
            pad: Some(2),
            seconds: Some(0.15),
            ..FireAction::new(FireActionKind::FirePad)
        }],
    );
    s.fire_cue(1).unwrap();
    pull(&mut s, 0, 10);
    s.poll();
    assert!(!playing(&s, 2), "not yet");
    std::thread::sleep(std::time::Duration::from_millis(200));
    s.poll();
    pull(&mut s, 0, 10);
    assert!(playing(&s, 2), "fired after the delay");
    s.poll();
    let crowd = s
        .log_entries()
        .into_iter()
        .find(|e| e.title == "Crowd")
        .unwrap();
    assert_eq!(crowd.source, "action");
}

#[test]
fn duck_others_for_a_while() {
    let (_b, mut s) = setup();
    with_actions(
        &mut s,
        1,
        vec![FireAction {
            db: Some(-20.0),
            seconds: Some(0.2),
            ..FireAction::new(FireActionKind::Duck)
        }],
    );
    s.fire_cue(0).unwrap();
    pull(&mut s, 0, 10);
    s.fire_cue(1).unwrap();
    pull(&mut s, 0, 48_000 / 2); // past the 0.15 s attack
    let ducked = pull(&mut s, 0, 1)[0];
    // Music at 0.5 × 0.1 (-20 dB) plus the horn at 0.25.
    assert!((ducked - (0.05 + 0.25)).abs() < 0.01, "{ducked}");
    std::thread::sleep(std::time::Duration::from_millis(250));
    s.poll();
    pull(&mut s, 0, 48_000); // the 0.8 s release
    let back = pull(&mut s, 0, 1)[0];
    assert!((back - 0.75).abs() < 0.01, "{back}");
}

#[test]
fn pads_that_fire_each_other_dont_loop_forever() {
    let (_b, mut s) = setup();
    with_actions(
        &mut s,
        0,
        vec![FireAction {
            pad: Some(1),
            ..FireAction::new(FireActionKind::FirePad)
        }],
    );
    with_actions(
        &mut s,
        1,
        vec![FireAction {
            pad: Some(0),
            ..FireAction::new(FireActionKind::FirePad)
        }],
    );
    s.fire_cue(0).unwrap(); // returns: nesting is capped
    pull(&mut s, 0, 10);
    assert!(playing(&s, 0) && playing(&s, 1));
}

#[test]
fn actions_follow_moved_pads_save_and_are_checked() {
    let (_b, mut s) = setup();
    with_actions(
        &mut s,
        1,
        vec![FireAction {
            pad: Some(2),
            ..FireAction::new(FireActionKind::FirePad)
        }],
    );
    s.move_cue(2, 3).unwrap();
    assert_eq!(s.show().cue(1).unwrap().on_fire[0].pad, Some(3));
    let back = sbj_session::Show::from_json(&s.show().to_json()).unwrap();
    assert_eq!(
        back.cue(1).unwrap().on_fire,
        s.show().cue(1).unwrap().on_fire
    );
    s.remove_cue(3).unwrap();
    assert!(
        s.preflight()
            .checks
            .iter()
            .any(|c| c.level == CheckLevel::Warning && c.message.contains("action")),
    );
}
