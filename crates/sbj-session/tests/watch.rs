//! Watch folders: new audio files join the library once they've finished
//! copying; duplicates, hidden and non-audio files, and files the
//! operator removed are left alone.

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::*;
use sbj_backend::offline::OfflineBackend;
use sbj_session::{Session, Show, WatchFolder};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sbj-watch-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn watching(dir: &Path, subfolders: bool) -> (OfflineBackend, Session<OfflineBackend>) {
    let backend = two_devices();
    let mut s = session(&backend);
    s.set_watch_interval(Duration::ZERO);
    s.set_watch_folders(vec![WatchFolder {
        path: dir.display().to_string(),
        subfolders,
    }]);
    (backend, s)
}

fn library(s: &Session<OfflineBackend>) -> Vec<String> {
    s.library_entries()
        .into_iter()
        .map(|e| {
            Path::new(&e.file)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

#[test]
fn new_files_are_added_once_they_stop_changing() {
    let dir = temp("new");
    let (_b, mut s) = watching(&dir, true);
    s.poll();
    assert!(library(&s).is_empty());

    write_wav(&dir.join("Walk-up.wav"), 48_000, 100, 1000);
    std::fs::create_dir_all(dir.join("Goals")).unwrap();
    write_wav(&dir.join("Goals/Horn.wav"), 48_000, 100, 1000);
    std::fs::write(dir.join("notes.txt"), "not audio").unwrap();
    std::fs::write(dir.join(".hidden.wav"), "x").unwrap();
    std::fs::write(dir.join("empty.wav"), "").unwrap();

    // Just written: wait one scan to be sure they're finished.
    s.poll();
    assert!(library(&s).is_empty());
    let imports = s.watch_imports();
    s.poll();
    assert_eq!(library(&s), ["Horn.wav", "Walk-up.wav"]);
    assert_eq!(s.watch_imports(), imports + 1);
    assert!(s.is_dirty());
    let notices = s.take_notices();
    assert!(notices.iter().any(|n| n.contains("2 files")), "{notices:?}");
    assert!(s.events().iter().any(|e| e.kind == "library"));

    // Nothing new: nothing happens.
    s.poll();
    assert_eq!(s.watch_imports(), imports + 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_still_being_copied_waits() {
    let dir = temp("growing");
    let (_b, mut s) = watching(&dir, true);
    write_wav(&dir.join("Big.wav"), 48_000, 100, 1000);
    s.poll();
    write_wav(&dir.join("Big.wav"), 48_000, 2000, 1000); // grew
    s.poll();
    assert!(library(&s).is_empty());
    s.poll();
    assert_eq!(library(&s), ["Big.wav"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn subfolders_can_be_left_out() {
    let dir = temp("flat");
    let (_b, mut s) = watching(&dir, false);
    std::fs::create_dir_all(dir.join("Old")).unwrap();
    write_wav(&dir.join("Old/Skip.wav"), 48_000, 100, 1000);
    write_wav(&dir.join("Take.wav"), 48_000, 100, 1000);
    s.poll();
    s.poll();
    assert_eq!(library(&s), ["Take.wav"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn removed_and_duplicate_files_are_not_added_again() {
    let dir = temp("dupes");
    let elsewhere = temp("dupes-elsewhere");
    // Already in the library from another folder (as after Save With Audio).
    write_wav(&elsewhere.join("Same.wav"), 48_000, 100, 1000);
    let (_b, mut s) = watching(&dir, true);
    s.add_to_library(vec![elsewhere.join("Same.wav").display().to_string()]);
    write_wav(&dir.join("Same.wav"), 48_000, 100, 1000);
    write_wav(&dir.join("Drop.wav"), 48_000, 100, 1000);
    s.poll();
    s.poll();
    assert_eq!(library(&s), ["Drop.wav", "Same.wav"]);
    assert_eq!(s.library_entries().len(), 2);

    let drop = dir.join("Drop.wav").display().to_string();
    s.remove_from_library(&drop).unwrap();
    s.poll();
    s.poll();
    assert_eq!(library(&s), ["Same.wav"]);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&elsewhere);
}

#[test]
fn a_missing_folder_is_logged_once_and_picked_up_when_it_appears() {
    let dir = temp("missing");
    std::fs::remove_dir_all(&dir).unwrap();
    let (_b, mut s) = watching(&dir, true);
    s.poll();
    s.poll();
    let warnings = s
        .events()
        .iter()
        .filter(|e| e.kind == "library" && e.level == "warning")
        .count();
    assert_eq!(warnings, 1);
    assert_eq!(s.missing_watch_folders(), [dir.display().to_string()]);

    std::fs::create_dir_all(&dir).unwrap();
    write_wav(&dir.join("Late.wav"), 48_000, 100, 1000);
    s.poll();
    assert!(s.missing_watch_folders().is_empty());
    s.poll();
    assert_eq!(library(&s), ["Late.wav"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn watch_folders_are_saved_with_the_show() {
    let (_b, mut s) = watching(Path::new("/tmp/clips/"), false);
    s.set_watch_folders(vec![
        WatchFolder {
            path: "/tmp/clips/".into(),
            subfolders: false,
        },
        WatchFolder {
            path: "/tmp/clips".into(),
            subfolders: true,
        },
        WatchFolder {
            path: "  ".into(),
            subfolders: true,
        },
    ]);
    assert_eq!(s.watch_folders().len(), 1);
    assert_eq!(s.watch_folders()[0].path, "/tmp/clips");
    let json = s.show().to_json();
    let back = Show::from_json(&json).unwrap();
    assert_eq!(back.watch_folders, s.watch_folders());
    // Old shows without the field still open.
    assert!(
        Show::from_json(&Show::default().to_json())
            .unwrap()
            .watch_folders
            .is_empty()
    );
}

#[test]
fn the_pregame_check_warns_about_missing_watch_folders() {
    let (_b, s) = watching(Path::new("/nonexistent/sbj/clips"), true);
    let checks = s.preflight().checks;
    assert!(
        checks
            .iter()
            .any(|c| c.message.contains("/nonexistent/sbj/clips")),
        "{checks:?}"
    );
}
