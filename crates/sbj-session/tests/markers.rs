//! Markers that fire or stop other pads, and volume markers.

mod common;

use common::*;
use sbj_session::{ActionDoc, ActionKind, CueDoc, DeviceRef, MarkerDoc, OutputDoc};

/// 0.01 s at 48 kHz.
const AT: usize = 480;

fn marker(id: u64, kind: ActionKind, pad: Option<u32>, seconds: Option<f64>) -> MarkerDoc {
    MarkerDoc {
        id,
        name: format!("m{id}"),
        time: AT as f64 / 48_000.0,
        action: ActionDoc {
            pad,
            seconds,
            ..ActionDoc::new(kind)
        },
    }
}

fn runs(out: &[f32], expected: &[(usize, f32)]) {
    let mut from = 0;
    for &(to, value) in expected {
        assert!(
            out[from..to].iter().all(|&s| (s - value).abs() < 1e-5),
            "expected {value} in {from}..{to}, got {:?}",
            &out[from..to.min(from + 8)]
        );
        from = to;
    }
}

/// Pad 0 ("Source", 0.25) with `markers`; pad 1 ("Target", 0.5, 1000
/// frames).
fn setup(
    markers: Vec<MarkerDoc>,
) -> (
    sbj_backend::offline::OfflineBackend,
    sbj_session::Session<sbj_backend::offline::OfflineBackend>,
) {
    let backend = two_devices();
    let mut s = session(&backend);
    s.add_sound("src".into(), constant(0.25, 48_000));
    s.add_sound("tgt".into(), constant(0.5, 1_000));
    let mut source = CueDoc::simple(0, "Source", "src");
    source.markers = markers;
    s.set_cue(source).unwrap();
    s.set_cue(CueDoc::simple(1, "Target", "tgt")).unwrap();
    (backend, s)
}

#[test]
fn fire_marker_starts_a_pad_on_the_same_output_at_the_exact_sample() {
    let (_b, mut s) = setup(vec![marker(1, ActionKind::Fire, Some(1), None)]);
    assert!(s.problems().is_empty(), "{:?}", s.problems());
    s.fire_cue(0).unwrap();
    let out = pull(&mut s, 0, 2_000);
    runs(&out, &[(AT, 0.25), (AT + 1_000, 0.75), (2_000, 0.25)]);

    // The session follows up: a log entry from "marker".
    let dir = tempfile::tempdir().unwrap();
    s.set_log_dir(Some(dir.path().to_path_buf()));
    s.fire_cue(0).unwrap();
    pull(&mut s, 0, AT + 10);
    s.poll();
    let entries = s.log_entries();
    let target = entries
        .iter()
        .find(|e| e.title == "Target")
        .expect("logged");
    assert_eq!(target.source, "marker");
}

#[test]
fn fired_pad_hands_over_its_exclusive_group() {
    let (_b, mut s) = setup(vec![marker(1, ActionKind::Fire, Some(1), None)]);
    s.add_sound("other".into(), constant(0.125, 48_000));
    let mut other = CueDoc::simple(2, "Other", "other");
    other.group = Some("Music".into());
    s.set_cue(other).unwrap();
    let mut target = s.show().cue(1).unwrap().clone();
    target.group = Some("music".into());
    s.set_cue(target).unwrap();

    s.fire_cue(2).unwrap();
    s.fire_cue(0).unwrap();
    pull(&mut s, 0, AT + 10);
    s.poll();
    pull(&mut s, 0, 48_000); // the group fade (1 s by default)
    assert!(
        !s.cue_state(2).unwrap().playing,
        "faded out by the hand-over"
    );
}

#[test]
fn stop_pad_marker_stops_a_pad_at_the_exact_sample() {
    let target_long = |seconds| {
        let (b, mut s) = setup(vec![marker(1, ActionKind::StopPad, Some(1), seconds)]);
        s.add_sound("long".into(), constant(0.5, 48_000));
        s.set_cue(CueDoc::simple(1, "Target", "long")).unwrap();
        s.fire_cue(1).unwrap();
        s.fire_cue(0).unwrap();
        let out = pull(&mut s, 0, 2_000);
        (b, s, out)
    };
    let (_b, s, out) = target_long(Some(0.0));
    runs(&out, &[(AT, 0.75), (2_000, 0.25)]);
    assert!(!s.cue_state(1).unwrap().playing);

    // Default: over the target's own fade-out (0.5 s here).
    let (_b, _s, out) = target_long(None);
    assert!((out[AT] - 0.75).abs() < 1e-5);
    assert!(out[AT + 100] < 0.75 && out[AT + 100] > 0.25);
}

#[test]
fn volume_marker_changes_this_pads_level() {
    let mut m = marker(1, ActionKind::Volume, None, None);
    m.action.db = Some(-6.020_6);
    let (_b, mut s) = setup(vec![m]);
    s.fire_cue(0).unwrap();
    let out = pull(&mut s, 0, 1_000);
    runs(&out, &[(AT, 0.25), (1_000, 0.125)]);
    // A fresh press starts at full level again.
    s.fire_cue(0).unwrap();
    runs(&pull(&mut s, 0, 100), &[(100, 0.25)]);
}

#[test]
fn pads_on_other_outputs_are_fired_and_stopped_by_the_session() {
    let (_b, mut s) = setup(vec![marker(1, ActionKind::Fire, Some(1), None)]);
    s.set_outputs(vec![
        OutputDoc::named("Main"),
        OutputDoc {
            name: "Concourse".into(),
            device: Some(DeviceRef {
                id: B.into(),
                name: String::new(),
            }),
            backup: None,
        },
    ])
    .unwrap();
    // Moving the target to the other output rebuilds the source's marker.
    let mut target = s.show().cue(1).unwrap().clone();
    target.output = 1;
    s.set_cue(target).unwrap();
    assert!(s.problems().is_empty(), "{:?}", s.problems());

    s.fire_cue(0).unwrap();
    let out = pull(&mut s, 0, 1_000);
    runs(&out, &[(1_000, 0.25)]);
    assert!(!s.cue_state(1).unwrap().playing);
    s.poll();
    pull(&mut s, 1, 10);
    assert!(s.cue_state(1).unwrap().playing, "fired on the poll");
}

#[test]
fn markers_follow_pads_that_move() {
    let (_b, mut s) = setup(vec![marker(1, ActionKind::Fire, Some(1), None)]);
    s.move_cue(1, 5).unwrap();
    assert_eq!(s.show().cue(0).unwrap().markers[0].action.pad, Some(5));
    s.fire_cue(0).unwrap();
    runs(&pull(&mut s, 0, 1_500), &[(AT, 0.25), (AT + 1_000, 0.75)]);

    // Renumbering banks keeps bank and pad.
    let banks = vec!["A".into(), "B".into()];
    s.set_show_settings("T", 12, 4, banks).unwrap();
    s.move_cue(5, 12 + 3).unwrap();
    s.set_show_settings("T", 8, 4, vec!["A".into(), "B".into()])
        .unwrap();
    assert_eq!(s.show().cue(0).unwrap().markers[0].action.pad, Some(8 + 3));
    assert_eq!(s.show().cue(8 + 3).unwrap().title, "Target");
}

#[test]
fn a_marker_on_an_empty_pad_is_reported() {
    let (_b, mut s) = setup(vec![marker(1, ActionKind::Fire, Some(7), None)]);
    assert!(
        s.problems()
            .iter()
            .any(|p| p.slot == 0 && p.message.contains("empty pad")),
        "{:?}",
        s.problems()
    );
    // Filling that pad fixes it.
    s.set_cue(CueDoc::simple(7, "Late", "tgt")).unwrap();
    assert!(s.problems().is_empty(), "{:?}", s.problems());
    s.fire_cue(0).unwrap();
    runs(&pull(&mut s, 0, 1_000), &[(AT, 0.25), (1_000, 0.75)]);
}
