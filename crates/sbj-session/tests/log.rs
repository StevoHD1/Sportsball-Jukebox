//! The play log: entries, sources, durations, totals and files.

mod common;

use common::*;
use sbj_session::{ControlMessage, CueDoc, PlaylistDoc};

#[test]
fn logs_what_played_from_where_and_for_how_long() {
    let dir = tempfile::tempdir().unwrap();
    let backend = two_devices();
    let mut s = session(&backend);
    s.set_log_dir(Some(dir.path().to_path_buf()));
    s.set_game("vs. Tigers");
    s.add_sound("horn".into(), constant(0.5, 100_000));
    let mut horn = CueDoc::simple(1, "Goal Horn", "horn");
    horn.sponsor = Some("Acme Bank".into());
    s.set_cue(horn).unwrap();

    s.fire_cue(1).unwrap();
    pull(&mut s, 0, 10);
    s.poll();
    let entries = s.log_entries();
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert!(e.open && e.title == "Goal Horn" && e.source == "operator");
    assert_eq!((e.bank, e.pad, e.game.as_str()), (1, 2, "vs. Tigers"));
    assert_eq!(e.sponsor.as_deref(), Some("Acme Bank"));

    std::thread::sleep(std::time::Duration::from_millis(30));
    s.stop_cue(1, 0.0).unwrap();
    pull(&mut s, 0, 10);
    s.poll();
    let e = &s.log_entries()[0];
    assert!(!e.open && e.seconds >= 0.03, "{}", e.seconds);

    // From OSC, and a restart while playing makes a second entry.
    s.handle_control(ControlMessage::Osc {
        address: "/sbj/fire/2".into(),
        args: vec![],
    });
    pull(&mut s, 0, 10);
    s.fire_cue(1).unwrap();
    pull(&mut s, 0, 10);
    s.poll();
    let entries = s.log_entries();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[1].source, "osc");
    assert!(!entries[1].open && entries[2].open);

    let (songs, sponsors) = s.log_totals();
    assert_eq!(songs[0].plays, 3);
    assert_eq!(sponsors[0].name, "Acme Bank");
    assert_eq!(sponsors[0].plays, 3);

    // Closed entries are on disk as JSON lines.
    let files: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(files.len(), 1);
    let text = std::fs::read_to_string(files[0].as_ref().unwrap().path()).unwrap();
    assert_eq!(text.lines().count(), 2, "two closed so far");
    let first: sbj_session::LogEntry = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert_eq!(first.title, "Goal Horn");
}

#[test]
fn playlist_songs_are_logged_individually_with_auto_source() {
    let backend = two_devices();
    let mut s = session(&backend);
    for (name, value) in [("a", 0.1), ("b", 0.2)] {
        s.add_sound(name.into(), constant(value, 4_800));
    }
    let mut c = CueDoc::simple(0, "Timeout", "unused");
    c.clips.clear();
    c.playlist = Some(PlaylistDoc {
        items: vec!["a".into(), "b".into()],
        shuffle: false,
        auto_advance: true,
        crossfade: 0.01,
    });
    s.set_cue(c).unwrap();
    s.fire_cue(0).unwrap();
    for _ in 0..40 {
        pull(&mut s, 0, 256);
        s.poll();
    }
    let entries = s.log_entries();
    assert!(entries.len() >= 3, "{}", entries.len());
    assert_eq!(
        (entries[0].file.as_str(), entries[0].source.as_str()),
        ("a", "operator")
    );
    assert_eq!(
        (entries[1].file.as_str(), entries[1].source.as_str()),
        ("b", "auto")
    );
    assert!(!entries[0].open, "closed when the next song took over");

    s.clear_log();
    assert!(s.log_entries().is_empty());
}
