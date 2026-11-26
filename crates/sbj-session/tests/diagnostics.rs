//! The event log: glitches with what was playing, device changes, failed
//! fires and notices; the glitch count reset; the daily file.

mod common;

use common::*;
use sbj_backend::OutputStream;
use sbj_session::CueDoc;

fn setup() -> (
    sbj_backend::offline::OfflineBackend,
    sbj_session::Session<sbj_backend::offline::OfflineBackend>,
) {
    let backend = two_devices();
    let mut s = session(&backend);
    s.add_sound("a".into(), constant(0.5, 96_000));
    s.set_cue(CueDoc::simple(0, "Walk-up", "a")).unwrap();
    (backend, s)
}

fn kinds(
    s: &sbj_session::Session<sbj_backend::offline::OfflineBackend>,
    kind: &str,
) -> Vec<String> {
    s.events()
        .iter()
        .filter(|e| e.kind == kind)
        .map(|e| e.message.clone())
        .collect()
}

#[test]
fn glitches_are_logged_with_what_was_playing_and_can_be_reset() {
    let (_b, mut s) = setup();
    assert!(
        kinds(&s, "device")
            .iter()
            .any(|m| m.contains("playing through")),
        "{:?}",
        s.events()
    );
    s.fire_cue(0).unwrap();
    pull(&mut s, 0, 512);

    s.output_stream_mut(0).unwrap().status().record_xrun();
    s.poll();
    let glitches = kinds(&s, "glitch");
    assert_eq!(glitches.len(), 1, "{glitches:?}");
    assert!(glitches[0].starts_with("A glitch"), "{}", glitches[0]);
    assert!(glitches[0].contains("\"Walk-up\""), "{}", glitches[0]);
    assert_eq!(s.outputs_state()[0].xruns, 1);

    // Logged once, not on every poll.
    s.poll();
    assert_eq!(kinds(&s, "glitch").len(), 1);

    s.stop_all(0.0).unwrap();
    pull(&mut s, 0, 512);
    s.output_stream_mut(0).unwrap().status().record_xrun();
    s.output_stream_mut(0).unwrap().status().record_xrun();
    s.poll();
    let last = kinds(&s, "glitch").pop().unwrap();
    assert!(last.starts_with("2 glitches"), "{last}");
    assert_eq!(s.outputs_state()[0].xruns, 3);

    s.reset_glitches();
    assert_eq!(s.outputs_state()[0].xruns, 0);
    s.output_stream_mut(0).unwrap().status().record_xrun();
    s.poll();
    assert_eq!(s.outputs_state()[0].xruns, 1);
}

#[test]
fn failed_fires_and_notices_are_logged() {
    let (backend, mut s) = setup();
    // No device left to play through.
    backend.unplug(A);
    backend.unplug(B);
    s.poll();
    assert!(s.fire_cue(0).is_err());
    let fires = kinds(&s, "fire");
    assert!(fires.iter().any(|m| m.contains("\"Walk-up\"")), "{fires:?}");
    assert!(
        s.events()
            .iter()
            .any(|e| e.level == "error" && e.kind == "device"),
        "{:?}",
        s.events()
    );
    assert!(
        s.events()
            .iter()
            .any(|e| e.level == "error" && e.kind == "fire"),
        "{:?}",
        s.events()
    );
}

#[test]
fn lost_devices_are_logged() {
    let (backend, mut s) = setup();
    backend.unplug(A);
    s.poll();
    let notices = s.take_notices();
    assert!(!notices.is_empty());
    // Every operator notice also lands in the log.
    for n in &notices {
        assert!(s.events().iter().any(|e| &e.message == n), "{n}");
    }
}

#[test]
fn events_are_appended_to_a_daily_file() {
    let (_b, mut s) = setup();
    let dir = std::env::temp_dir().join(format!("sbj-events-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    s.set_log_dir(Some(dir.clone()));
    s.output_stream_mut(0).unwrap().status().record_xrun();
    s.poll();
    let file = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .find(|e| e.file_name().to_string_lossy().starts_with("events-"))
        .expect("an events file");
    let text = std::fs::read_to_string(file.path()).unwrap();
    let line: serde_json::Value = serde_json::from_str(text.lines().last().unwrap()).unwrap();
    assert_eq!(line["kind"], "glitch");
    assert_eq!(line["level"], "warning");
    assert!(line["time"].as_f64().unwrap() > 1.0e9);
    let _ = std::fs::remove_dir_all(&dir);
}
