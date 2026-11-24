//! Banks, MIDI/OSC control, MIDI learn and the pre-game check.

mod common;

use common::*;
use sbj_session::control::OscArg;
use sbj_session::{
    CheckLevel, ControlMessage, CueDoc, LearnTarget, MidiKind, MidiMessage, MidiTrigger,
};

fn banks(n: usize) -> Vec<String> {
    (1..=n).map(|i| format!("Bank {i}")).collect()
}

fn osc(address: &str) -> ControlMessage {
    ControlMessage::Osc {
        address: address.into(),
        args: Vec::new(),
    }
}

fn osc_with(address: &str, args: Vec<OscArg>) -> ControlMessage {
    ControlMessage::Osc {
        address: address.into(),
        args,
    }
}

fn note(number: u8, channel: u8) -> ControlMessage {
    ControlMessage::Midi(MidiMessage::parse(&[0x90 | (channel - 1), number, 100]).unwrap())
}

/// A session with 2 banks of 4 pads: "a" in bank 1 pad 1, "b" in bank 2
/// pad 3.
fn banked() -> (
    sbj_backend::offline::OfflineBackend,
    sbj_session::Session<sbj_backend::offline::OfflineBackend>,
) {
    let backend = two_devices();
    let mut s = session(&backend);
    s.set_show_settings("Test", 4, 4, banks(2)).unwrap();
    s.add_sound("a".into(), constant(0.25, 10_000));
    s.add_sound("b".into(), constant(0.5, 10_000));
    s.set_cue(CueDoc::simple(0, "Alpha", "a")).unwrap();
    s.set_cue(CueDoc::simple(6, "Beta", "b")).unwrap();
    (backend, s)
}

#[test]
fn midi_parsing() {
    let on = MidiMessage::parse(&[0x92, 60, 100]).unwrap();
    assert_eq!((on.kind, on.number, on.channel), (MidiKind::Note, 60, 3));
    assert!(
        MidiMessage::parse(&[0x92, 60, 0]).is_none(),
        "note-on velocity 0 = off"
    );
    assert!(MidiMessage::parse(&[0x82, 60, 100]).is_none(), "note off");
    assert_eq!(
        MidiMessage::parse(&[0xB0, 7, 127]).unwrap().kind,
        MidiKind::Cc
    );
    assert!(MidiMessage::parse(&[0xB0, 7, 0]).is_none(), "CC release");
    assert_eq!(MidiMessage::parse(&[0xC5, 12]).unwrap().number, 12);
    assert!(MidiMessage::parse(&[0xF8]).is_none(), "clock");
    assert!(
        MidiMessage::parse(&[0x90, 200, 1]).is_none(),
        "bad data byte"
    );
    assert!(MidiMessage::parse(&[]).is_none());
}

#[test]
fn banks_renumber_and_switch() {
    let (_b, mut s) = banked();
    assert_eq!(s.show().total_slots(), 8);
    // Growing the bank size keeps cues at the same bank/pad.
    s.set_show_settings("Test", 6, 3, banks(3)).unwrap();
    assert_eq!(s.show().cue(0).unwrap().title, "Alpha");
    assert_eq!(s.show().cue(6 + 2).unwrap().title, "Beta", "bank 2 pad 3");
    s.fire_cue(8).unwrap();
    assert!(pull(&mut s, 0, 10).iter().all(|&x| x == 0.5));
    // Removing a bank that holds a cue is refused.
    assert!(s.set_show_settings("Test", 6, 3, banks(1)).is_err());
    s.set_bank(2).unwrap();
    s.step_bank(true);
    assert_eq!(s.current_bank(), 0, "wraps");
    s.step_bank(false);
    assert_eq!(s.current_bank(), 2);
    assert!(s.set_bank(3).is_err());
}

#[test]
fn osc_fires_stops_and_switches_banks() {
    let (_b, mut s) = banked();
    s.handle_control(osc("/sbj/fire/1"));
    assert!(
        s.last_control().unwrap().contains("fire \"Alpha\""),
        "{:?}",
        s.last_control()
    );
    assert!(pull(&mut s, 0, 10).iter().all(|&x| x == 0.25));

    // Bank 2 pad 3, explicitly and via the current bank.
    s.handle_control(osc("/sbj/fire/2/3"));
    assert!(pull(&mut s, 0, 10).iter().all(|&x| x == 0.75));
    s.handle_control(osc("/sbj/stopall/now"));
    pull(&mut s, 0, 10);
    s.handle_control(osc("/sbj/bank/2"));
    assert_eq!(s.current_bank(), 1);
    s.handle_control(osc_with("/sbj/fire", vec![OscArg::Int(3)]));
    assert!(pull(&mut s, 0, 10).iter().all(|&x| x == 0.5));

    // Button release (0) is ignored; title lookup; stop.
    s.handle_control(osc_with("/sbj/stop/3", vec![OscArg::Float(0.0)]));
    assert!(s.cue_state(6).unwrap().playing);
    s.handle_control(osc_with("/SBJ/STOP/3", vec![OscArg::Float(1.0)]));
    pull(&mut s, 0, 10);
    assert!(!s.cue_state(6).unwrap().playing);
    s.handle_control(osc_with("/sbj/title", vec![OscArg::Str("alpha".into())]));
    assert!(s.cue_state(0).unwrap().playing || pull(&mut s, 0, 1)[0] > 0.0);

    s.handle_control(osc("/sbj/fire/9"));
    assert!(s.last_control().unwrap().contains("no such pad"));
    s.handle_control(osc("/nope"));
    assert!(s.last_control().unwrap().contains("unknown"));
}

#[test]
fn midi_learn_and_trigger() {
    let (_b, mut s) = banked();
    assert!(s.start_learning(LearnTarget::Cue(3)).is_err(), "empty pad");
    s.start_learning(LearnTarget::Cue(6)).unwrap();
    s.handle_control(note(60, 1));
    assert_eq!(s.learning(), None);
    assert_eq!(
        s.show().cue(6).unwrap().midi,
        Some(MidiTrigger {
            kind: MidiKind::Note,
            number: 60,
            channel: Some(1)
        })
    );
    assert!(s.is_dirty());

    s.handle_control(note(60, 2));
    assert!(
        s.last_control().unwrap().contains("not assigned"),
        "other channel"
    );
    s.handle_control(note(60, 1));
    assert!(s.last_control().unwrap().contains("fire \"Beta\""));
    assert!(pull(&mut s, 0, 10).iter().all(|&x| x == 0.5));

    // Show-wide controls.
    s.start_learning(LearnTarget::StopAll).unwrap();
    s.handle_control(note(1, 16));
    s.start_learning(LearnTarget::NextBank).unwrap();
    s.handle_control(ControlMessage::Midi(
        MidiMessage::parse(&[0xB0, 20, 127]).unwrap(),
    ));
    s.handle_control(ControlMessage::Midi(
        MidiMessage::parse(&[0xB0, 20, 127]).unwrap(),
    ));
    assert_eq!(s.current_bank(), 1);
    s.handle_control(note(1, 16));
    pull(&mut s, 0, 48_000);
    assert!(!s.cue_state(6).unwrap().playing, "stop all faded it out");

    s.set_control_trigger(LearnTarget::Cue(6), None);
    assert_eq!(s.show().cue(6).unwrap().midi, None);
}

#[test]
fn preflight_reports_problems() {
    let (backend, mut s) = banked();
    let ok = s.preflight();
    assert!(ok.ready(), "{:?}", ok.checks);
    assert!(ok.memory_bytes >= 2 * 10_000 * 4);

    // Duplicate key in the same bank, a missing file, a dead output.
    let mut c = s.show().cue(0).unwrap().clone();
    c.hotkey = Some("x".into());
    s.set_cue(c).unwrap();
    let mut c = CueDoc::simple(1, "Gamma", "/nope/gone.wav");
    c.hotkey = Some("X".into());
    s.set_cue(c).unwrap();
    let report = s.preflight();
    assert!(!report.ready());
    let text: Vec<String> = report.checks.iter().map(|c| c.message.clone()).collect();
    assert!(text.iter().any(|m| m.contains("Key \"X\"")), "{text:?}");
    assert!(text.iter().any(|m| m.contains("gone.wav")), "{text:?}");
    assert_eq!(report.checks[0].level, CheckLevel::Error, "errors first");

    backend.unplug(A);
    backend.unplug(B);
    s.poll();
    let report = s.preflight();
    assert!(
        report
            .checks
            .iter()
            .any(|c| c.message.contains("no audio device"))
    );
    assert!(
        report
            .checks
            .iter()
            .any(|c| c.message.contains("cannot play"))
    );
}
