//! The show's audio library, previewing library files, defaults for new
//! pads, pad pictures and custom colours.

mod common;

use common::*;
use sbj_session::{CueDoc, DeviceRef, OutputDoc, PadDefaults, PlaylistDoc, Show};

#[test]
fn library_lists_kept_and_used_files() {
    let backend = two_devices();
    let mut s = session(&backend);
    s.add_sound("used.wav".into(), constant(0.5, 48_000));
    s.set_cue(CueDoc::simple(3, "Used", "used.wav")).unwrap();
    let mut playlist = CueDoc::simple(4, "Music", "x");
    playlist.clips.clear();
    playlist.playlist = Some(PlaylistDoc {
        items: vec!["used.wav".into(), "song.mp3".into()],
        shuffle: false,
        auto_advance: true,
        crossfade: 1.0,
    });
    s.set_cue(playlist).unwrap();

    assert_eq!(
        s.add_to_library(vec![
            "kept.wav".into(),
            "used.wav".into(),
            "kept.wav".into()
        ]),
        1,
        "files a pad uses, and repeats, are not added twice"
    );
    let entries = s.library_entries();
    let files: Vec<&str> = entries.iter().map(|e| e.file.as_str()).collect();
    assert_eq!(files, ["kept.wav", "song.mp3", "used.wav"]);
    let used = entries.iter().find(|e| e.file == "used.wav").unwrap();
    assert_eq!(used.used_by, [3, 4]);
    assert!(used.loaded);
    assert_eq!(used.duration_seconds, Some(1.0));
    let kept = entries.iter().find(|e| e.file == "kept.wav").unwrap();
    assert!(!kept.loaded && kept.used_by.is_empty() && kept.duration_seconds.is_none());

    assert!(s.remove_from_library("used.wav").is_err(), "a pad uses it");
    s.remove_from_library("kept.wav").unwrap();
    assert!(s.show().library.is_empty());

    // Saved with the show.
    s.add_to_library(vec!["kept.wav".into()]);
    let back = Show::from_json(&s.show().to_json()).unwrap();
    assert_eq!(back.library, ["kept.wav"]);
}

#[test]
fn library_files_preview_on_the_preview_output() {
    let backend = two_devices();
    let mut s = session(&backend);
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
    s.add_sound("song.wav".into(), constant(0.5, 48_000));
    assert!(s.preview_file("song.wav").is_err(), "no preview output yet");
    s.set_preview_output(Some(1)).unwrap();
    assert!(s.preview_file("missing.wav").is_err(), "not loaded");

    s.preview_file("song.wav").unwrap();
    let headphones = pull(&mut s, 1, 100);
    assert!(headphones[50] > 0.4, "{}", headphones[50]);
    assert!(
        pull(&mut s, 0, 100).iter().all(|&x| x == 0.0),
        "not on the arena"
    );
    let state = s.preview_state().unwrap();
    assert_eq!(state.file.as_deref(), Some("song.wav"));
    assert_eq!(state.slot, sbj_session::FILE_PREVIEW);

    s.preview_file("song.wav").unwrap(); // again: stop
    pull(&mut s, 1, 1_000);
    assert!(s.preview_state().is_none());
}

#[test]
fn pad_defaults_images_and_custom_colours_are_saved() {
    let backend = two_devices();
    let mut s = session(&backend);
    let defaults = PadDefaults {
        color: Some("#12ab34".into()),
        fade_in: 0.25,
        group: Some("Music".into()),
        trim_silence: false,
        ..PadDefaults::default()
    };
    s.set_pad_defaults(Some(defaults.clone()));
    assert!(s.is_dirty());
    s.add_sound("a".into(), constant(0.5, 100));
    let mut pad = CueDoc::simple(0, "Horn", "a");
    pad.color = Some("#FF8800".into());
    pad.image = Some("horn.png".into());
    s.set_cue(pad).unwrap();

    let json = s.show().to_json();
    assert!(json.contains("\"trim_silence\": false"), "{json}");
    let back = Show::from_json(&json).unwrap();
    assert_eq!(back.pad_defaults, Some(defaults));
    assert_eq!(back.cues[0].color.as_deref(), Some("#FF8800"));
    assert_eq!(back.cues[0].image.as_deref(), Some("horn.png"));

    // Built-in defaults are stored as nothing.
    s.set_pad_defaults(Some(PadDefaults::default()));
    assert_eq!(s.show().pad_defaults, None);
}

#[test]
fn pictures_are_found_in_an_images_folder_beside_the_show() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("Images")).unwrap();
    std::fs::write(dir.path().join("Images").join("horn.png"), b"png").unwrap();
    let found =
        sbj_session::show::resolve_file("/Users/someone/Pictures/horn.png", Some(dir.path()));
    assert_eq!(found, Some(dir.path().join("Images").join("horn.png")));
    assert_eq!(
        sbj_session::show::resolve_file("missing.png", Some(dir.path())),
        None
    );
}
