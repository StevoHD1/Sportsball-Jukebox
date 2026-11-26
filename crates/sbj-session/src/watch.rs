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
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
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
    /// Download files that are only in the cloud (Box Drive, iCloud and
    /// other File Provider folders) so they're on this Mac.
    #[serde(default = "yes")]
    pub download: bool,
}

/// Downloads running at once.
const MAX_DOWNLOADS: usize = 4;

/// A file that still looks online-only after being downloaded (Box gave
/// its space back, or the download failed) isn't fetched again
/// automatically for this long.
const REFETCH_AFTER: Duration = Duration::from_secs(600);

/// Whether a file is a cloud placeholder whose contents aren't on this
/// Mac yet (macOS "dataless" files, as Box Drive and iCloud leave them).
/// Reading it makes the cloud app download it.
pub fn is_online_only(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt;
        /// `SF_DATALESS` from `<sys/stat.h>`.
        const SF_DATALESS: u32 = 0x4000_0000;
        std::fs::symlink_metadata(path).is_ok_and(|m| m.st_flags() & SF_DATALESS != 0)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        false
    }
}

/// Reads a whole file, which makes a cloud app download it.
fn fetch(path: &Path) -> std::io::Result<u64> {
    let mut file = std::fs::File::open(path)?;
    let mut buffer = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        match file.read(&mut buffer)? {
            0 => return Ok(total),
            n => total += n as u64,
        }
    }
}

/// Background downloads of online-only files (not reset with the show).
#[derive(Debug)]
pub(crate) struct Downloads {
    sender: mpsc::Sender<(PathBuf, std::io::Result<u64>)>,
    receiver: mpsc::Receiver<(PathBuf, std::io::Result<u64>)>,
    active: HashSet<PathBuf>,
    /// When each file was last fetched automatically.
    fetched: HashMap<PathBuf, Instant>,
    /// Counts starts and finishes (the app reloads the library).
    pub(crate) changes: u64,
    /// For tests: which files count as online-only.
    pub(crate) probe: fn(&Path) -> bool,
}

impl Default for Downloads {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            sender,
            receiver,
            active: HashSet::new(),
            fetched: HashMap::new(),
            changes: 0,
            probe: is_online_only,
        }
    }
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

    /// Whether a library file is only in the cloud (not on this Mac).
    pub fn is_file_online_only(&self, file: &str) -> bool {
        self.resolve_library_file(file)
            .is_some_and(|p| (self.downloads.probe)(&p))
    }

    /// Whether a library file is being downloaded.
    pub fn is_downloading(&self, file: &str) -> bool {
        self.resolve_library_file(file)
            .is_some_and(|p| self.downloads.active.contains(&p))
    }

    /// Counts download starts and finishes (reload the library when it
    /// changes).
    pub fn download_changes(&self) -> u64 {
        self.downloads.changes
    }

    /// For tests: decide which files count as online-only.
    pub fn set_online_only_probe(&mut self, probe: fn(&Path) -> bool) {
        self.downloads.probe = probe;
    }

    fn resolve_library_file(&self, file: &str) -> Option<PathBuf> {
        resolve_file(file, self.show_dir().as_deref())
    }

    /// Downloads a library file that's only in the cloud, in the
    /// background (reading it makes Box Drive or iCloud fetch it).
    pub fn download_file(&mut self, file: &str) -> crate::Result<()> {
        let path = self.resolve_library_file(file).ok_or_else(|| {
            crate::SessionError::Show(crate::ShowError::Invalid(format!("{file} can't be found")))
        })?;
        self.start_download(path);
        Ok(())
    }

    fn start_download(&mut self, path: PathBuf) {
        if !self.downloads.active.insert(path.clone()) {
            return;
        }
        let sender = self.downloads.sender.clone();
        let spawned = std::thread::Builder::new()
            .name("sbj-download".into())
            .spawn({
                let path = path.clone();
                move || {
                    let result = fetch(&path);
                    let _ = sender.send((path, result));
                }
            });
        if let Err(e) = spawned {
            self.downloads.active.remove(&path);
            self.log_event(
                "error",
                "library",
                format!("Couldn't start downloading \"{}\": {e}", path.display()),
            );
            return;
        }
        self.downloads.changes += 1;
        self.log_event(
            "info",
            "library",
            format!("Downloading \"{}\" from the cloud.", path.display()),
        );
    }

    /// Reports finished downloads (called from `poll`).
    pub(crate) fn finish_downloads(&mut self) {
        while let Ok((path, result)) = self.downloads.receiver.try_recv() {
            self.downloads.active.remove(&path);
            self.downloads.changes += 1;
            let name = path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            match result {
                Ok(bytes) => self.log_event(
                    "info",
                    "library",
                    format!(
                        "Downloaded \"{name}\" ({:.1} MB).",
                        bytes as f64 / 1_048_576.0
                    ),
                ),
                Err(e) => self.log_event(
                    "warning",
                    "library",
                    format!("Couldn't download \"{}\": {e}", path.display()),
                ),
            }
        }
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
            let first = found.len();
            let result = list(Path::new(&folder.path), folder.subfolders, 0, &mut found);
            if folder.download {
                let probe = self.downloads.probe;
                let downloads = &self.downloads;
                let recent = |p: &PathBuf| {
                    downloads
                        .fetched
                        .get(p)
                        .is_some_and(|t| now.duration_since(*t) < REFETCH_AFTER)
                };
                let wanted: Vec<PathBuf> = found[first..]
                    .iter()
                    .map(|(p, ..)| p.clone())
                    .filter(|p| !downloads.active.contains(p) && !recent(p) && probe(p))
                    .collect();
                for path in wanted {
                    if self.downloads.active.len() >= MAX_DOWNLOADS {
                        break; // the rest on later scans
                    }
                    self.downloads.fetched.insert(path.clone(), now);
                    self.start_download(path);
                }
            }
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
