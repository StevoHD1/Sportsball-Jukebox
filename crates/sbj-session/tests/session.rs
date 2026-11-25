//! End-to-end: show → session → engines → (offline) devices.

mod common;

use std::time::Duration;

use common::*;
use sbj_session::{
    ActionDoc, ActionKind, ClipDoc, CueDoc, DecodeReport, DeviceRef, MarkerDoc, OutputDoc,
    OutputStatus, Session, SessionError, Show, TargetDoc, decode_files, waveform,
};

fn simple(slot: u32, file: &str) -> CueDoc {
    CueDoc::simple(slot, file, file)
}

#[test]
fn fire_plays_cue_to_completion() {
    let backend = two_devices();
    let mut s = session(&backend);
    s.add_sound("a".into(), constant(0.5, 1000));
    s.set_cue(simple(0, "a")).unwrap();
    s.fire_cue(0).unwrap();
    let out = pull(&mut s, 0, 1500);
    assert!(out[..1000].iter().all(|&x| x == 0.5));
    assert!(out[1000..].iter().all(|&x| x == 0.0));
    let st = s.cue_state(0).unwrap();
    assert!(st.assigned && st.ready && !st.playing);
    assert!((st.duration_seconds - 1000.0 / 48_000.0).abs() < 1e-9);
}

#[test]
fn cues_overlap_and_empty_pads_do_nothing() {
    let backend = two_devices();
    let mut s = session(&backend);
    s.add_sound("a".into(), constant(0.25, 1000));
    s.add_sound("b".into(), constant(0.5, 1000));
    s.set_cue(simple(0, "a")).unwrap();
    s.set_cue(simple(7, "b")).unwrap();
    s.fire_cue(0).unwrap();
    s.fire_cue(7).unwrap();
    s.fire_cue(3).unwrap();
    assert!(pull(&mut s, 0, 100).iter().all(|&x| x == 0.75));
}

#[test]
fn timeline_cue_with_marker_loop_by_marker_reference() {
    let backend = two_devices();
    let mut s = session(&backend);
    s.add_sound("r".into(), ramp(48_000));
    // Play 0.0-0.5 s; marker "vamp" at 0.25 s; marker at 0.5 s loops back
    // to "vamp" twice, then continues to the end at 1.0 s... but the clip
    // is only 1 s long and starts at 0.
    let mut cue = simple(2, "r");
    cue.markers = vec![
        MarkerDoc {
            id: 1,
            name: "vamp".into(),
            time: 0.25,
            action: ActionDoc {
                kind: ActionKind::Jump,
                to: Some(TargetDoc {
                    time: Some(0.25),
                    marker: None,
                }),
                count: None,
                seconds: None,
                hold: false,
                pad: None,
                db: None,
            },
        },
        MarkerDoc {
            id: 2,
            name: "back".into(),
            time: 0.5,
            action: ActionDoc {
                kind: ActionKind::Loop,
                to: Some(TargetDoc {
                    time: None,
                    marker: Some(1),
                }),
                count: Some(2),
                seconds: None,
                hold: false,
                pad: None,
                db: None,
            },
        },
    ];
    s.set_cue(cue).unwrap();
    assert!(s.problems().is_empty(), "{:?}", s.problems());
    s.fire_cue(2).unwrap();
    let out = pull(&mut s, 0, 48_000 + 2 * 12_000 + 100);
    let mut expected = ramp_values(0..24_000);
    expected.extend(ramp_values(12_000..24_000));
    expected.extend(ramp_values(12_000..24_000));
    expected.extend(ramp_values(24_000..48_000));
    assert_eq!(&out[..expected.len()], expected.as_slice());
    assert!(out[expected.len()..].iter().all(|&x| x == 0.0));
}

#[test]
fn clips_are_placed_trimmed_and_faded() {
    let backend = two_devices();
    let mut s = session(&backend);
    s.add_sound("r".into(), ramp(48_000));
    let mut cue = simple(0, "r");
    cue.clips = vec![ClipDoc {
        start: 0.01,        // 480 frames of silence first
        offset: 0.1,        // from frame 4800
        length: Some(0.02), // 960 frames
        ..ClipDoc::file("r")
    }];
    s.set_cue(cue).unwrap();
    s.fire_cue(0).unwrap();
    let out = pull(&mut s, 0, 2000);
    assert!(out[..480].iter().all(|&x| x == 0.0));
    assert_eq!(&out[480..1440], ramp_values(4800..5760).as_slice());
    assert!(out[1440..].iter().all(|&x| x == 0.0));
}

#[test]
fn whole_cue_loop() {
    let backend = two_devices();
    let mut s = session(&backend);
    s.add_sound("a".into(), constant(0.5, 100));
    let mut cue = simple(0, "a");
    cue.looping = true;
    s.set_cue(cue).unwrap();
    s.fire_cue(0).unwrap();
    assert!(pull(&mut s, 0, 1000).iter().all(|&x| x == 0.5));
    assert!(s.cue_state(0).unwrap().looping && s.cue_state(0).unwrap().playing);
}

#[test]
fn cues_route_to_their_output_and_convert_rate() {
    let backend = two_devices();
    let mut s = session(&backend);
    s.set_outputs(vec![
        OutputDoc::named("Main"),
        OutputDoc {
            name: "Locker room".into(),
            device: Some(DeviceRef {
                id: B.into(),
                name: String::new(),
            }),
            backup: None,
        },
    ])
    .unwrap();
    s.add_sound("a".into(), constant(0.5, 48_000));
    let mut cue = simple(0, "a");
    cue.output = 1;
    s.set_cue(cue).unwrap();
    s.fire_cue(0).unwrap();

    assert!(
        pull(&mut s, 0, 1000).iter().all(|&x| x == 0.0),
        "not on Main"
    );
    // One second at 48 kHz plays as one second at 44.1 kHz on device B.
    let locker = pull(&mut s, 1, 45_000);
    let audible = locker.iter().filter(|x| x.abs() > 0.01).count();
    assert!((audible as i64 - 44_100).abs() < 50, "{audible}");

    let outputs = s.outputs_state();
    assert_eq!(outputs[1].device_id.as_deref(), Some(B));
    assert_eq!(outputs[1].sample_rate, 44_100);
}

#[test]
fn failover_to_backup_resumes_playback() {
    let backend = two_devices();
    let mut s = Session::new(backend.clone());
    s.set_declick(0.0);
    s.install_show(
        Show {
            outputs: vec![OutputDoc {
                name: "Main".into(),
                device: Some(DeviceRef {
                    id: A.into(),
                    name: String::new(),
                }),
                backup: Some(DeviceRef {
                    id: B.into(),
                    name: String::new(),
                }),
            }],
            ..Show::default()
        },
        None,
        DecodeReport::default(),
    );
    s.add_sound("r".into(), ramp(96_000));
    s.set_cue(simple(0, "r")).unwrap();
    s.fire_cue(0).unwrap();
    pull(&mut s, 0, 24_000); // 0.5 s in

    backend.unplug(A);
    s.poll();

    let out = &s.outputs_state()[0];
    assert_eq!(out.status, OutputStatus::OnBackup);
    assert_eq!(out.device_id.as_deref(), Some(B));
    let notices = s.take_notices();
    assert!(notices.iter().any(|n| n.contains("lost")), "{notices:?}");

    // Resumes from about where it was (rates differ, so allow a frame or
    // two of rounding).
    let resumed = pull(&mut s, 0, 441);
    assert!(
        (resumed[0] - 24_000.0 / 1e6).abs() < 0.0005,
        "{}",
        resumed[0]
    );
    let st = s.cue_state(0).unwrap();
    assert!(st.playing);
    assert!(
        (st.position_seconds - 0.51).abs() < 0.001,
        "{}",
        st.position_seconds
    );
}

#[test]
fn offline_output_retries_until_a_device_returns() {
    let backend = two_devices();
    let mut s = session(&backend);
    s.set_retry_interval(Duration::ZERO);
    s.add_sound("a".into(), constant(0.5, 1000));
    s.set_cue(simple(0, "a")).unwrap();

    backend.unplug(A);
    backend.unplug(B);
    s.poll();
    assert_eq!(s.outputs_state()[0].status, OutputStatus::Offline);
    assert!(matches!(s.fire_cue(0), Err(SessionError::NoOutput)));
    assert!(!s.cue_state(0).unwrap().ready);

    let mut a = sbj_backend::offline::OfflineDevice::new(A, 48_000, 2);
    a.is_default = true;
    backend.plug(a);
    s.poll();
    assert_eq!(s.outputs_state()[0].status, OutputStatus::Running);
    s.fire_cue(0).unwrap();
    assert!(pull(&mut s, 0, 10).iter().all(|&x| x == 0.5));
}

#[test]
fn removing_an_output_moves_its_cues_to_the_first() {
    let backend = two_devices();
    let mut s = session(&backend);
    s.set_outputs(vec![OutputDoc::named("Main"), OutputDoc::named("Second")])
        .unwrap();
    s.add_sound("a".into(), constant(0.5, 1000));
    let mut cue = simple(0, "a");
    cue.output = 1;
    s.set_cue(cue).unwrap();
    s.set_outputs(vec![OutputDoc::named("Main")]).unwrap();
    assert_eq!(s.show().cue(0).unwrap().output, 0);
    s.fire_cue(0).unwrap();
    assert!(pull(&mut s, 0, 10).iter().all(|&x| x == 0.5));
}

#[test]
fn move_cue_swaps_pads() {
    let backend = two_devices();
    let mut s = session(&backend);
    s.add_sound("a".into(), constant(0.25, 1000));
    s.add_sound("b".into(), constant(0.5, 1000));
    s.set_cue(simple(0, "a")).unwrap();
    s.set_cue(simple(1, "b")).unwrap();
    s.move_cue(0, 1).unwrap();
    assert_eq!(s.show().cue(0).unwrap().title, "b");
    s.fire_cue(0).unwrap();
    assert!(pull(&mut s, 0, 10).iter().all(|&x| x == 0.5));
    s.move_cue(1, 5).unwrap();
    assert!(s.show().cue(1).is_none());
    assert_eq!(s.show().cue(5).unwrap().title, "a");
}

#[test]
fn play_from_and_stop_all() {
    let backend = two_devices();
    let mut s = session(&backend);
    s.add_sound("r".into(), ramp(48_000));
    s.set_cue(simple(0, "r")).unwrap();
    s.play_cue_from(0, 0.5).unwrap();
    assert_eq!(pull(&mut s, 0, 3), ramp_values(24_000..24_003));
    s.stop_all(0.01).unwrap();
    let out = pull(&mut s, 0, 1000);
    assert!(out[480..].iter().all(|&x| x == 0.0));
    assert!(!s.cue_state(0).unwrap().playing);
}

#[test]
fn errors_and_limits() {
    let backend = two_devices();
    let mut s = session(&backend);
    assert!(matches!(s.fire_cue(99), Err(SessionError::InvalidCue(99))));
    let mut cue = simple(0, "a");
    cue.output = 3;
    assert!(matches!(
        s.set_cue(cue),
        Err(SessionError::InvalidOutput(3))
    ));
    s.add_sound("a".into(), constant(0.5, 10));
    s.set_cue(simple(20, "a")).unwrap();
    assert!(
        s.set_show_settings("x", 10, 5, vec!["Bank 1".into()])
            .is_err(),
        "pad 21 in use"
    );
    s.set_show_settings("Opener", 30, 6, vec!["Bank 1".into()])
        .unwrap();
    assert_eq!(s.show().slots, 30);
    assert!(s.is_dirty());
}

#[test]
fn missing_files_are_reported_not_fatal() {
    let backend = two_devices();
    let mut s = session(&backend);
    s.add_sound("a".into(), constant(0.5, 100));
    let mut cue = simple(0, "a");
    cue.clips.push(ClipDoc::file("/nope/missing.wav"));
    s.set_cue(cue).unwrap();
    assert_eq!(s.problems().len(), 1);
    assert!(s.problems()[0].message.contains("missing.wav"));
    assert!(
        s.problems()[0].message.contains("not found"),
        "{:?}",
        s.problems()
    );
    s.fire_cue(0).unwrap();
    assert!(
        pull(&mut s, 0, 10).iter().all(|&x| x == 0.5),
        "the rest still plays"
    );
}

#[test]
fn shows_save_load_and_find_moved_audio() {
    let dir = tempfile::tempdir().unwrap();
    let audio = dir.path().join("Audio");
    std::fs::create_dir(&audio).unwrap();
    write_wav(&audio.join("horn.wav"), 44_100, 4410, 8000);

    let mut show = Show {
        name: "Opener".into(),
        ..Show::default()
    };
    // Stored with a path from "another Mac"; found in Audio/ next to the show.
    let mut cue = CueDoc::simple(4, "Horn", "/Users/someone/Desktop/horn.wav");
    cue.hotkey = Some("h".into());
    cue.gain = 0.5;
    show.cues.push(cue);

    let path = dir.path().join("opener.sbjshow");
    show.save(&path).unwrap();
    let loaded = Show::load(&path).unwrap();
    assert_eq!(loaded, show);

    let backend = two_devices();
    let decoded = decode_files(&loaded.files(), Some(dir.path()), &[48_000]);
    assert!(decoded.failures.is_empty(), "{:?}", decoded.failures);
    let mut s = Session::new(backend);
    s.set_declick(0.0);
    s.install_show(loaded, Some(path.clone()), decoded);
    assert!(s.problems().is_empty(), "{:?}", s.problems());
    s.fire_cue(4).unwrap();
    let out = pull(&mut s, 0, 100);
    assert!(out[50] > 0.1 && out[50] < 0.15, "{}", out[50]); // 8000/32768 * 0.5

    // Saving round-trips and clears the dirty flag.
    s.set_show_settings("Renamed", 24, 6, vec!["Bank 1".into()])
        .unwrap();
    s.save_show(&path).unwrap();
    assert!(!s.is_dirty());
    assert_eq!(Show::load(&path).unwrap().name, "Renamed");
}

#[test]
fn show_json_compatibility() {
    // Minimal file: everything else defaults.
    let show = Show::from_json(r#"{"format":"sportsball-jukebox-show","version":1}"#).unwrap();
    assert_eq!(show.slots, 24);
    assert_eq!(show.outputs.len(), 1);

    assert!(Show::from_json(r#"{"format":"other","version":1}"#).is_err());
    let too_new = Show::from_json(r#"{"format":"sportsball-jukebox-show","version":99}"#);
    assert!(too_new.unwrap_err().to_string().contains("newer version"));
    let dup = r#"{"format":"sportsball-jukebox-show","version":1,
        "cues":[{"slot":1,"clips":[]},{"slot":1,"clips":[]}]}"#;
    assert!(Show::from_json(dup).is_err());

    // Snake-case keys and compact defaults, as the Swift app expects.
    let mut show = Show::default();
    let mut cue = CueDoc::simple(0, "t", "f.wav");
    cue.looping = true;
    cue.markers.push(MarkerDoc {
        id: 1,
        name: "m".into(),
        time: 1.0,
        action: ActionDoc {
            kind: ActionKind::FadeOut,
            to: None,
            count: None,
            seconds: Some(2.0),
            hold: false,
            pad: None,
            db: None,
        },
    });
    show.cues.push(cue);
    let json = show.to_json();
    assert!(json.contains("\"loop\": true"));
    assert!(json.contains("\"kind\": \"fade_out\""));
    assert!(json.contains("\"fade_curve\": \"linear\""));
    assert!(!json.contains("\"gain\""), "default gain omitted");
    assert_eq!(Show::from_json(&json).unwrap(), show);
}

#[test]
fn waveform_peaks() {
    let sound = ramp(1000);
    let peaks = waveform(sound.original(), 10);
    assert_eq!(peaks.len(), 10);
    assert!((peaks[9].1 - 1000.0 / 1e6).abs() < 1e-9);
    assert_eq!(peaks[0].0, 0.0);
}

#[test]
fn polling_keeps_event_queues_drained() {
    let backend = two_devices();
    let mut s = session(&backend);
    s.add_sound("a".into(), constant(0.5, 10));
    s.set_cue(simple(0, "a")).unwrap();
    for _ in 0..10_000 {
        s.fire_cue(0).unwrap();
        pull(&mut s, 0, 20);
        s.poll();
    }
    let _ = s.outputs_state();
}
