//! The show's audio library: every file the show keeps for building
//! pads, whether or not a pad uses it yet. Files only in the library are
//! not decoded until they are previewed or put on a pad, so a big library
//! does not fill memory.

use sbj_backend::AudioBackend;

use crate::show::PadDefaults;
use crate::{Result, Session, SessionError, ShowError};

/// One library file, for display.
#[derive(Debug, Clone, PartialEq)]
pub struct LibraryEntry {
    pub file: String,
    /// Slots of the pads that use it (clips or playlist songs).
    pub used_by: Vec<u32>,
    /// Decoded (so duration and loudness are known).
    pub loaded: bool,
    pub duration_seconds: Option<f64>,
    pub loudness_lufs: Option<f64>,
    /// Could not be decoded, and why.
    pub error: Option<String>,
}

impl<B: AudioBackend> Session<B> {
    /// The library, sorted by file.
    pub fn library_entries(&self) -> Vec<LibraryEntry> {
        self.show
            .library_files()
            .into_iter()
            .map(|file| {
                let used_by = self
                    .show
                    .cues
                    .iter()
                    .filter(|c| c.files().contains(&file))
                    .map(|c| c.slot)
                    .collect();
                let sound = self.library.get(&file);
                LibraryEntry {
                    used_by,
                    loaded: sound.is_some(),
                    duration_seconds: sound.map(|s| s.duration_seconds()),
                    loudness_lufs: sound.and_then(|s| s.analysis().loudness_lufs),
                    error: self.file_errors.get(&file).cloned(),
                    file,
                }
            })
            .collect()
    }

    /// Adds files to the library (without decoding them). Returns how many
    /// were new.
    pub fn add_to_library(&mut self, files: Vec<String>) -> usize {
        let known = self.show.library_files();
        let mut added = 0;
        for file in files {
            let file = file.trim().to_string();
            if file.is_empty() || known.contains(&file) || self.show.library.contains(&file) {
                continue;
            }
            self.show.library.push(file);
            added += 1;
        }
        if added > 0 {
            self.show.library.sort();
            self.dirty = true;
        }
        added
    }

    /// Removes a file from the library. Files a pad uses cannot be
    /// removed (clear the pad first).
    pub fn remove_from_library(&mut self, file: &str) -> Result<()> {
        if self
            .show
            .cues
            .iter()
            .any(|c| c.files().iter().any(|f| f == file))
        {
            return Err(SessionError::Show(ShowError::Invalid(format!(
                "{file} is used by a pad"
            ))));
        }
        let before = self.show.library.len();
        self.show.library.retain(|f| f != file);
        if self.show.library.len() != before {
            self.dirty = true;
        }
        Ok(())
    }

    /// Settings for new pads (`None` = the built-in defaults).
    pub fn set_pad_defaults(&mut self, defaults: Option<PadDefaults>) {
        let defaults = defaults.filter(|d| *d != PadDefaults::default());
        if self.show.pad_defaults != defaults {
            self.show.pad_defaults = defaults;
            self.dirty = true;
        }
    }
}
