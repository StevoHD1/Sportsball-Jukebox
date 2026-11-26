//! "Save With Audio": copy every file the show uses into one folder, so
//! the folder can be moved to another Mac or a USB stick and just work.
//!
//! Done in two steps so the slow part runs without holding the session:
//! [`Session::collect_plan`] works out what to copy where (quick), the
//! caller copies the files ([`CollectPlan::copy`]), then
//! [`Session::apply_collect`] points the show at the copies (relative to
//! the folder, as `Audio/…` and `Images/…`) before it is saved there.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

use sbj_backend::AudioBackend;

use crate::Session;
use crate::show::resolve_file;

/// What "Save With Audio" will do.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct CollectPlan {
    /// Files to copy: source, destination.
    pub copies: Vec<(PathBuf, PathBuf)>,
    /// How each file string in the show will be written afterwards.
    pub renames: HashMap<String, String>,
    /// Files the show refers to that could not be found.
    pub missing: Vec<String>,
}

impl CollectPlan {
    /// Copies the files (creating the folders). Slow; call without
    /// holding the session. Files already at their destination are left.
    pub fn copy(&self) -> io::Result<()> {
        for (from, to) in &self.copies {
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)?;
            }
            if !to.exists() {
                // Copy to a temporary name first, so a failed copy never
                // leaves a half-written file the show would point at.
                let partial = to.with_extension("sbjpart");
                std::fs::copy(from, &partial)?;
                std::fs::rename(&partial, to)?;
            }
        }
        Ok(())
    }

    /// Bytes to copy (for showing progress up front).
    pub fn bytes(&self) -> u64 {
        self.copies
            .iter()
            .filter(|(_, to)| !to.exists())
            .filter_map(|(from, _)| std::fs::metadata(from).ok())
            .map(|m| m.len())
            .sum()
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

impl<B: AudioBackend> Session<B> {
    /// Plans copying the show's audio (pads and library) into
    /// `folder/Audio` and pad pictures into `folder/Images`.
    pub fn collect_plan(&self, folder: &Path) -> CollectPlan {
        let dir = self.show_dir();
        let mut plan = CollectPlan::default();
        let mut by_source: HashMap<PathBuf, String> = HashMap::new();
        let mut taken: HashSet<String> = HashSet::new();

        let images: Vec<String> = self
            .show
            .cues
            .iter()
            .filter_map(|c| c.image.clone())
            .collect();
        let files = self
            .show
            .library_files()
            .into_iter()
            .map(|f| (f, "Audio"))
            .chain(images.into_iter().map(|f| (f, "Images")));

        for (file, sub) in files {
            if plan.renames.contains_key(&file) {
                continue;
            }
            let Some(source) = resolve_file(&file, dir.as_deref()) else {
                plan.missing.push(file);
                continue;
            };
            let key = source.canonicalize().unwrap_or_else(|_| source.clone());
            if let Some(relative) = by_source.get(&key) {
                plan.renames.insert(file, relative.clone());
                continue;
            }
            let name = source
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "file".into());
            let (stem, ext) = match name.rsplit_once('.') {
                Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), format!(".{ext}")),
                _ => (name.clone(), String::new()),
            };
            // A free name in the subfolder: "Horn.wav", "Horn 2.wav", …
            let mut n = 1;
            let relative = loop {
                let candidate = if n == 1 {
                    format!("{sub}/{stem}{ext}")
                } else {
                    format!("{sub}/{stem} {n}{ext}")
                };
                let dest = folder.join(&candidate);
                let free =
                    !taken.contains(&candidate) && (!dest.exists() || same_file(&dest, &source));
                if free {
                    break candidate;
                }
                n += 1;
            };
            taken.insert(relative.clone());
            let dest = folder.join(&relative);
            if !same_file(&dest, &source) {
                plan.copies.push((source.clone(), dest));
            }
            by_source.insert(key, relative.clone());
            plan.renames.insert(file, relative);
        }
        plan.missing.sort();
        plan.missing.dedup();
        plan
    }

    /// Points the show at the collected copies (after [`CollectPlan::copy`]
    /// succeeded). Decoded audio is kept under the new names, so nothing
    /// reloads and nothing playing stops.
    pub fn apply_collect(&mut self, plan: &CollectPlan) {
        let rename = |file: &mut String| {
            if let Some(new) = plan.renames.get(file.as_str()) {
                *file = new.clone();
            }
        };
        for cue in &mut self.show.cues {
            cue.clips.iter_mut().for_each(|c| rename(&mut c.file));
            if let Some(playlist) = &mut cue.playlist {
                playlist.items.iter_mut().for_each(rename);
            }
            if let Some(image) = &mut cue.image {
                rename(image);
            }
        }
        self.show.library.iter_mut().for_each(rename);
        self.show.library.sort();
        self.show.library.dedup();
        for (old, new) in &plan.renames {
            if let Some(sound) = self.library.remove(old) {
                self.library.insert(new.clone(), sound);
            }
            if let Some(error) = self.file_errors.remove(old) {
                self.file_errors.insert(new.clone(), error);
            }
        }
        if !plan.missing.is_empty() {
            self.notify(format!(
                "{} file(s) could not be found and were not copied: {}",
                plan.missing.len(),
                plan.missing.join(", ")
            ));
        }
        self.dirty = true;
        // Older steps refer to the files by their old paths, which the
        // decoded audio is no longer filed under: start the history afresh.
        self.reset_undo();
    }
}
