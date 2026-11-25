//! The game rundown: GO, standby, notes, control surfaces.

mod common;

use common::*;
use sbj_session::{CheckLevel, ControlMessage, CueDoc, LearnTarget, MidiMessage, RundownItem};

fn pad(id: u64, slot: u32) -> RundownItem {
    RundownItem {
        id,
        pad: Some(slot),
        text: String::new(),
    }
}

fn note(id: u64, text: &str) -> RundownItem {
    RundownItem {
        id,
        pad: None,
        text: text.into(),
    }
}

/// Pads 0 "Anthem" (0.25), 1 "Lineups" (0.5), 2 "Horn" (0.125) and the
/// rundown: note, Anthem, Lineups, note, Horn.
fn setup() -> (
    sbj_backend::offline::OfflineBackend,
    sbj_session::Session<sbj_backend::offline::OfflineBackend>,
) {
    let backend = two_devices();
    let mut s = session(&backend);
    for (slot, title, value) in [(0, "Anthem", 0.25), (1, "Lineups", 0.5), (2, "Horn", 0.125)] {
        let file = title.to_lowercase();
        s.add_sound(file.clone(), constant(value, 48_000));
        s.set_cue(CueDoc::simple(slot, title, &file)).unwrap();
    }
    s.set_rundown(vec![
        note(1, "Pregame"),
        pad(2, 0),
        pad(3, 1),
        note(4, "Period 1"),
        pad(5, 2),
    ])
    .unwrap();
    (backend, s)
}

fn level(s: &mut sbj_session::Session<sbj_backend::offline::OfflineBackend>) -> f32 {
    pull(s, 0, 10)[9]
}

#[test]
fn go_fires_each_pad_step_in_order_skipping_notes() {
    let (_b, mut s) = setup();
    assert_eq!(
        s.standby(),
        Some(1),
        "starts on the first pad, not the note"
    );
    assert_eq!(s.rundown_go().unwrap(), Some(0));
    assert_eq!(level(&mut s), 0.25);
    assert_eq!(s.standby(), Some(2));
    s.rundown_go().unwrap();
    assert_eq!(level(&mut s), 0.75);
    assert_eq!(s.standby(), Some(4), "past the note");
    assert_eq!(s.rundown_go().unwrap(), Some(2));
    assert_eq!(s.standby(), None, "finished");
    assert_eq!(s.rundown_go().unwrap(), None, "GO at the end does nothing");

    // Rundown plays are logged as such.
    let dir = tempfile::tempdir().unwrap();
    s.set_log_dir(Some(dir.path().to_path_buf()));
    s.set_standby(Some(0));
    assert_eq!(s.standby(), Some(1), "a note moves on to the next pad");
    s.rundown_go().unwrap();
    pull(&mut s, 0, 10);
    s.poll();
    assert_eq!(s.log_entries().last().unwrap().source, "rundown");
}

#[test]
fn standby_steps_without_firing() {
    let (_b, mut s) = setup();
    s.step_standby(true);
    assert_eq!(s.standby(), Some(2));
    s.step_standby(true);
    assert_eq!(s.standby(), Some(4));
    s.step_standby(true);
    assert_eq!(s.standby(), None);
    s.step_standby(false);
    assert_eq!(s.standby(), Some(4), "back from finished");
    s.step_standby(false);
    s.step_standby(false);
    s.step_standby(false);
    assert_eq!(s.standby(), Some(1), "stays on the first");
    assert_eq!(level(&mut s), 0.0, "nothing fired");
    assert_eq!(s.rundown_label(1).as_deref(), Some("Anthem"));
    assert_eq!(s.rundown_label(0).as_deref(), Some("Pregame"));
}

#[test]
fn editing_keeps_standby_on_the_same_step() {
    let (_b, mut s) = setup();
    s.step_standby(true); // Lineups (id 3)
    let mut items = s.rundown().to_vec();
    items.insert(0, note(9, "Doors"));
    s.set_rundown(items.clone()).unwrap();
    assert_eq!(s.standby(), Some(3));
    assert_eq!(s.rundown_label(3).as_deref(), Some("Lineups"));
    // Removing it moves standby to the first pad step.
    items.retain(|i| i.id != 3);
    s.set_rundown(items).unwrap();
    assert_eq!(s.standby(), Some(2));
    // Duplicate ids are refused.
    assert!(s.set_rundown(vec![pad(1, 0), pad(1, 1)]).is_err());
}

#[test]
fn a_failed_go_stays_on_standby() {
    let (backend, mut s) = setup();
    backend.unplug(A);
    s.poll(); // notices the device is gone
    assert!(s.rundown_go().is_err());
    assert_eq!(s.standby(), Some(1), "GO can be pressed again");
}

#[test]
fn steps_follow_pads_that_move_and_empty_pads_are_flagged() {
    let (_b, mut s) = setup();
    s.move_cue(1, 7).unwrap();
    assert_eq!(s.rundown()[2].pad, Some(7));
    s.remove_cue(2).unwrap();
    let report = s.preflight();
    assert!(
        report
            .checks
            .iter()
            .any(|c| c.level == CheckLevel::Warning && c.message.contains("Rundown step 5")),
        "{:?}",
        report.checks
    );
    // Saved with the show.
    let json = s.show().to_json();
    assert!(json.contains("\"rundown\""));
    let back = sbj_session::Show::from_json(&json).unwrap();
    assert_eq!(back.rundown, s.rundown());
}

#[test]
fn go_from_osc_midi_and_the_remote() {
    let (_b, mut s) = setup();
    s.handle_control(ControlMessage::Osc {
        address: "/sbj/go".into(),
        args: vec![],
    });
    assert!(
        s.last_control().unwrap().contains("GO \"Anthem\""),
        "{:?}",
        s.last_control()
    );
    assert_eq!(s.standby(), Some(2));

    s.handle_control(ControlMessage::Osc {
        address: "/sbj/rundown/next".into(),
        args: vec![],
    });
    assert_eq!(s.standby(), Some(4));
    s.handle_control(ControlMessage::Osc {
        address: "/sbj/rundown/back".into(),
        args: vec![],
    });
    assert_eq!(s.standby(), Some(2));

    let note_on = || ControlMessage::Midi(MidiMessage::parse(&[0x90, 60, 100]).unwrap());
    s.start_learning(LearnTarget::RundownGo).unwrap();
    s.handle_control(note_on());
    assert!(s.show().controls.rundown_go.is_some());
    s.handle_control(note_on());
    assert_eq!(s.standby(), Some(4), "MIDI GO fired Lineups");

    let state = s.handle_remote("GET", "/api/state").unwrap();
    assert_eq!(state["rundown"]["standby"], "Horn");
    assert_eq!(state["rundown"]["step"], 3);
    assert_eq!(state["rundown"]["steps"], 3);
    let state = s.handle_remote("POST", "/api/go").unwrap();
    assert!(state["rundown"]["standby"].is_null(), "finished");
    s.handle_remote("POST", "/api/rundown/prev").unwrap();
    assert_eq!(s.standby(), Some(4));
}
