//! Undo / redo of show edits, and Save With Audio.

mod common;

use common::*;
use sbj_session::{CueDoc, Show, decode_files};

fn two_pads() -> (
    sbj_backend::offline::OfflineBackend,
    sbj_session::Session<sbj_backend::offline::OfflineBackend>,
) {
    let backend = two_devices();
    let mut s = session(&backend);
    s.add_sound("a".into(), constant(0.5, 480_000));
    s.add_sound("b".into(), constant(0.25, 480_000));
    s.set_cue(CueDoc::simple(0, "A", "a")).unwrap();
    s.set_cue(CueDoc::simple(1, "B", "b")).unwrap();
    s.record_undo("Set Up");
    (backend, s)
}

#[test]
fn undo_and_redo_restore_the_show_step_by_step() {
    let (_b, mut s) = two_pads();
    assert_eq!(s.undo_label(), Some("Set Up"));
    assert!(!s.record_undo("Nothing"), "no change, no step");

    let mut a = s.show().cue(0).unwrap().clone();
    a.title = "Renamed".into();
    a.color = Some("#FF0000".into());
    s.set_cue(a).unwrap();
    assert!(s.record_undo("Paste Style"));
    s.move_cue(1, 5).unwrap();
    assert!(s.record_undo("Move Pad"));
    assert_eq!(s.undo_label(), Some("Move Pad"));

    assert_eq!(s.undo().as_deref(), Some("Move Pad"));
    assert_eq!(s.show().cue(1).unwrap().title, "B");
    assert!(s.show().cue(5).is_none());
    assert_eq!(s.undo().as_deref(), Some("Paste Style"));
    assert_eq!(s.show().cue(0).unwrap().title, "A");
    assert_eq!(s.redo_label(), Some("Paste Style"));

    assert_eq!(s.redo().as_deref(), Some("Paste Style"));
    assert_eq!(s.show().cue(0).unwrap().title, "Renamed");
    // A new edit clears what could be redone.
    s.remove_cue(0).unwrap();
    s.record_undo("Clear Pad");
    assert_eq!(s.redo_label(), None);
    s.undo();
    assert_eq!(s.show().cue(0).unwrap().title, "Renamed", "the pad is back");

    // The restored pad plays.
    s.fire_cue(0).unwrap();
    assert!(pull(&mut s, 0, 10).iter().all(|&x| x == 0.5));
}

#[test]
fn undo_keeps_untouched_pads_playing() {
    let (_b, mut s) = two_pads();
    s.fire_cue(1).unwrap();
    pull(&mut s, 0, 100);
    let mut a = s.show().cue(0).unwrap().clone();
    a.title = "Changed".into();
    s.set_cue(a).unwrap();
    s.record_undo("Rename");
    s.undo();
    pull(&mut s, 0, 100);
    assert!(
        s.cue_state(1).unwrap().playing,
        "B was not touched by the undo"
    );
}

#[test]
fn unrecorded_edits_are_kept_by_undo() {
    let (_b, mut s) = two_pads();
    let mut a = s.show().cue(0).unwrap().clone();
    a.title = "First".into();
    s.set_cue(a.clone()).unwrap();
    s.record_undo("First");
    a.title = "Second".into();
    s.set_cue(a).unwrap(); // not recorded yet
    assert_eq!(
        s.undo().as_deref(),
        Some("Change"),
        "the unrecorded edit is undone first"
    );
    assert_eq!(s.show().cue(0).unwrap().title, "First");
    assert_eq!(s.undo().as_deref(), Some("First"));
}

#[test]
fn opening_a_show_clears_the_history() {
    let (_b, mut s) = two_pads();
    assert!(s.undo_label().is_some());
    s.new_show();
    assert_eq!(s.undo_label(), None);
    assert_eq!(s.undo(), None);
}

#[test]
fn save_with_audio_makes_a_self_contained_folder() {
    let source = tempfile::tempdir().unwrap();
    let other = source.path().join("other");
    std::fs::create_dir(&other).unwrap();
    // Two different files with the same name, a library-only file, a
    // picture, and one file that doesn't exist.
    for (dir, name, value) in [
        (source.path(), "horn.wav", 26_000i16),
        (other.as_path(), "horn.wav", 13_000),
        (source.path(), "extra.wav", 3_000),
    ] {
        write_wav(&dir.join(name), 48_000, 48_000, value);
    }
    std::fs::write(source.path().join("logo.png"), b"png").unwrap();
    let path = |p: &std::path::Path| p.display().to_string();
    let horn = path(&source.path().join("horn.wav"));
    let horn2 = path(&other.join("horn.wav"));
    let extra = path(&source.path().join("extra.wav"));

    let backend = two_devices();
    let mut s = session(&backend);
    let report = decode_files(&[horn.clone(), horn2.clone()], None, &[48_000]);
    for (file, sound) in report.sounds {
        s.add_sound(file, sound);
    }
    let mut a = CueDoc::simple(0, "Horn", &horn);
    a.image = Some(path(&source.path().join("logo.png")));
    s.set_cue(a).unwrap();
    s.set_cue(CueDoc::simple(1, "Other horn", &horn2)).unwrap();
    s.add_to_library(vec![extra, "/nowhere/missing.wav".into()]);
    s.fire_cue(0).unwrap();
    pull(&mut s, 0, 10);

    let dest = tempfile::tempdir().unwrap();
    let plan = s.collect_plan(dest.path());
    assert_eq!(plan.copies.len(), 4, "{:?}", plan.copies);
    assert_eq!(plan.missing, ["/nowhere/missing.wav"]);
    plan.copy().unwrap();
    s.apply_collect(&plan);
    s.save_show(&dest.path().join("Game.sbjshow")).unwrap();

    let saved = s.show();
    let a = saved.cue(0).unwrap();
    assert_eq!(a.clips[0].file, "Audio/horn.wav");
    assert_eq!(a.image.as_deref(), Some("Images/logo.png"));
    assert_eq!(saved.cue(1).unwrap().clips[0].file, "Audio/horn 2.wav");
    assert!(saved.library.contains(&"Audio/extra.wav".to_string()));
    assert!(s.take_notices().iter().any(|n| n.contains("missing.wav")));
    // Still playing, and the decoded audio follows the new names.
    assert!(s.cue_state(0).unwrap().playing);
    s.fire_cue(1).unwrap();
    let both = pull(&mut s, 0, 10);
    assert!(both.iter().all(|&x| x > 1.0), "both horns play: {both:?}");

    // Move the folder: everything opens from the new place.
    let moved = tempfile::tempdir().unwrap();
    let new_home = moved.path().join("Game");
    std::fs::rename(dest.path(), &new_home)
        .or_else(|_| copy_dir(dest.path(), &new_home))
        .unwrap();
    let show = Show::load(&new_home.join("Game.sbjshow")).unwrap();
    for file in show
        .library_files()
        .into_iter()
        .filter(|f| !f.contains("missing"))
    {
        assert!(
            sbj_session::show::resolve_file(&file, Some(&new_home)).is_some(),
            "{file}"
        );
    }
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}
