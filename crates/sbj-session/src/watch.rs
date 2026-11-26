//! Watch folders: audio files that appear in them (e.g. clips exported
//! during the game) are added to the show's library automatically.
//!
//! Folders are scanned from `poll` every couple of seconds. A file is
//! added once it has finished copying (its size and modification time
//! are the same on two scans, or it was last modified a few seconds
//! ago). Files already in the library (the same path, or the same name
//! and size, as after Save With Audio) are skipped. Files stay where they
//! are, and the folder is the source of truth: a file removed from the
//! library comes back on the next scan while it's still in the folder.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use sbj_backend::AudioBackend;
use serde::{Deserialize, Serialize};

use crate::Session;
use crate::show::resolve_file;

/// Audio files picked up (the app's own import list).
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "wav", "wave", "aif", "aiff", "aifc", "caf", "flac", "mp3", "m4a", "aac", "mp4", "alac", "ogg",
    "oga",
];

/// Time between scans.
pub(crate) const DEFAULT_WATCH_INTERVAL: Duration = Duration::from_secs(2);

/// Files modified longer ago than this are taken as finished.
const SETTLED: Duration = Duration::from_secs(3);

/// How deep "include subfolders" goes.
const MAX_DEPTH: usize = 8;

/// A folder whose new audio files are added to the library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchFolder {
    pub path: String,
    /// Also watch folders inside it.
    #[serde(default = "yes")]
    pub subfolders: bool,
}

fn yes() -> bool {
    true
}

/// Scanning state (not saved).
#[derive(Debug, Default)]
pub(crate) struct WatchState {
    next_scan: Option<Instant>,
    /// Files still being written: size and modification time last scan.
    pending: HashMap<PathBuf, (u64, Option<SystemTime>)>,
    /// Files already added or found in the library.
    seen: HashSet<PathBuf>,
    /// Folders that couldn't be read (logged once).
    missing: HashSet<String>,
    /// Imports so far (the app reloads the library when it changes).
    pub(crate) imports: u64,
}

fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

fn hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with('.') || n.starts_with('~'))
}

/// Audio files in `dir` (and below, with `deep`), with size and mtime.
fn list(
    dir: &Path,
    deep: bool,
    depth: usize,
    out: &mut Vec<(PathBuf, u64, Option<SystemTime>)>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if hidden(&path) {
            continue;
        }
        // Not following links avoids loops.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if deep && depth < MAX_DEPTH {
                let _ = list(&path, deep, depth + 1, out);
            }
        } else if kind.is_file() && is_audio(&path) {
            if let Ok(meta) = entry.metadata() {
                out.push((path, meta.len(), meta.modified().ok()));
            }
        }
    }
    Ok(())
}

impl<B: AudioBackend> Session<B> {
    /// The show's watch folders.
    pub fn watch_folders(&self) -> &[WatchFolder] {
        &self.show.watch_folders
    }

    /// Replaces the watch folders (blank and repeated paths are dropped)
    /// and scans them straight away.
    pub fn set_watch_folders(&mut self, folders: Vec<WatchFolder>) {
        let mut clean: Vec<WatchFolder> = Vec::new();
        for mut folder in folders {
            folder.path = folder.path.trim().trim_end_matches('/').to_string();
            if folder.path.is_empty() {
                // "/" trimmed to nothing: refuse to watch the whole disk.
                continue;
            }
            if !clean.iter().any(|f| f.path == folder.path) {
                clean.push(folder);
            }
        }
        if clean != self.show.watch_folders {
            self.show.watch_folders = clean;
            self.dirty = true;
        }
        self.watch
            .missing
            .retain(|p| self.show.watch_folders.iter().any(|f| &f.path == p));
        self.watch.next_scan = None;
    }

    /// Watch folders that can't be read right now.
    pub fn missing_watch_folders(&self) -> Vec<String> {
        let mut missing: Vec<String> = self.watch.missing.iter().cloned().collect();
        missing.sort();
        missing
    }

    /// How many times files have been added from watch folders (the app
    /// reloads the library when this changes).
    pub fn watch_imports(&self) -> u64 {
        self.watch.imports
    }

    /// A file left the library: offer it again if it's still watched.
    pub(crate) fn forget_watched(&mut self, file: &str) {
        let path = PathBuf::from(file);
        let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
        self.watch
            .seen
            .retain(|p| p != &path && p.canonicalize().map_or(true, |c| c != canonical));
    }

    /// For tests: scan more often.
    pub fn set_watch_interval(&mut self, interval: Duration) {
        self.watch_interval = interval;
    }

    /// Forgets which files were seen (a show was opened).
    pub(crate) fn reset_watch(&mut self) {
        let imports = self.watch.imports;
        self.watch = WatchState {
            imports,
            ..WatchState::default()
        };
    }

    /// Scans the watch folders if it's time (called from `poll`).
    pub(crate) fn scan_watch_folders(&mut self) {
        if self.show.watch_folders.is_empty() {
            return;
        }
        let now = Instant::now();
        if self.watch.next_scan.is_some_and(|t| now < t) {
            return;
        }
        self.watch.next_scan = Some(now + self.watch_interval);

        let mut found = Vec::new();
        for folder in self.show.watch_folders.clone() {
            let result = list(Path::new(&folder.path), folder.subfolders, 0, &mut found);
            match result {
                Ok(()) => {
                    if self.watch.missing.remove(&folder.path) {
                        self.log_event(
                            "info",
                            "library",
                            format!("Watching \"{}\" again.", folder.path),
                        );
                    }
                }
                Err(e) => {
                    if self.watch.missing.insert(folder.path.clone()) {
                        self.log_event(
                            "warning",
                            "library",
                            format!("Can't read the watch folder \"{}\": {e}", folder.path),
                        );
                    }
                }
            }
        }

        let candidates: Vec<_> = found
            .into_iter()
            .filter(|(path, ..)| !self.watch.seen.contains(path))
            .collect();
        let present: HashSet<&PathBuf> = candidates.iter().map(|(p, ..)| p).collect();
        self.watch.pending.retain(|p, _| present.contains(p));
        if candidates.is_empty() {
            return;
        }

        // What the library already holds, by path and by (name, size).
        let show_dir = self.show_dir();
        let mut known_paths = HashSet::new();
        let mut known_names = HashSet::new();
        for file in self.show.library_files() {
            if let Some(path) = resolve_file(&file, show_dir.as_deref()) {
                let size = std::fs::metadata(&path).map(|m| m.len()).ok();
                if let (Some(name), Some(size)) = (path.file_name(), size) {
                    known_names.insert((name.to_os_string(), size));
                }
                known_paths.insert(path.canonicalize().unwrap_or(path));
            }
        }

        let wall = SystemTime::now();
        let mut new_files = Vec::new();
        for (path, size, modified) in candidates {
            let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
            let by_name = path
                .file_name()
                .is_some_and(|n| known_names.contains(&(n.to_os_string(), size)));
            if known_paths.contains(&canonical) || by_name {
                self.watch.pending.remove(&path);
                self.watch.seen.insert(path);
                continue;
            }
            let old = modified
                .and_then(|m| wall.duration_since(m).ok())
                .is_some_and(|age| age >= SETTLED);
            let unchanged = self.watch.pending.get(&path) == Some(&(size, modified));
            if size > 0 && (old || unchanged) {
                self.watch.pending.remove(&path);
                self.watch.seen.insert(path.clone());
                new_files.push(path.to_string_lossy().into_owned());
            } else {
                self.watch.pending.insert(path, (size, modified));
            }
        }
        if new_files.is_empty() {
            return;
        }
        new_files.sort();
        let names: Vec<String> = new_files
            .iter()
            .map(|f| {
                Path::new(f)
                    .file_name()
                    .map_or_else(|| f.clone(), |n| n.to_string_lossy().into_owned())
            })
            .collect();
        let added = self.add_to_library(new_files);
        if added == 0 {
            return;
        }
        self.watch.imports += 1;
        let message = if added == 1 {
            format!("Added \"{}\" to the library from a watch folder.", names[0])
        } else {
            format!(
                "Added {added} files to the library from watch folders: {}",
                names.join(", ")
            )
        };
        self.log_event("info", "library", message.clone());
        self.notices.push(message);
    }
}
