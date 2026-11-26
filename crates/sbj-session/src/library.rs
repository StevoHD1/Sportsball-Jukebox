//! The show's audio library: every file the show keeps for building
//! pads, whether or not a pad uses it yet. Files only in the library are
//! not decoded until they are previewed or put on a pad, so a big library
//! does not fill memory.

use sbj_backend::AudioBackend;

use crate::show::{BankDefaults, PadDefaults, ShowColor};
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
    /// Only in the cloud (Box Drive, iCloud), not on this Mac yet.
    pub online_only: bool,
    pub downloading: bool,
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
                    online_only: sound.is_none() && self.is_file_online_only(&file),
                    downloading: self.is_downloading(&file),
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
        // Files are managed in the Finder: a file still in a watch folder
        // comes back on the next scan.
        self.forget_watched(file);
        Ok(())
    }

    /// Replaces the show's colour presets. Entries without a name or a
    /// valid "#RRGGBB" are dropped. With `recolor`, pads (and the new-pad
    /// default) using a preset whose colour changed, matched by name, get
    /// the new colour. Returns how many pads were recoloured.
    pub fn set_show_colors(&mut self, colors: Vec<ShowColor>, recolor: bool) -> usize {
        let colors: Vec<ShowColor> = colors
            .into_iter()
            .filter_map(|c| {
                let name = c.name.trim().to_string();
                let value = ShowColor::normalize(&c.value)?;
                (!name.is_empty()).then_some(ShowColor { name, value })
            })
            .collect();
        let mut recolored = 0;
        if recolor {
            for new in &colors {
                let Some(old) = self.show.colors.iter().find(|o| o.name == new.name) else {
                    continue;
                };
                if old.value == new.value {
                    continue;
                }
                let old_value = old.value.clone();
                let matches = |c: &Option<String>| {
                    c.as_deref()
                        .and_then(ShowColor::normalize)
                        .is_some_and(|v| v == old_value)
                };
                for cue in &mut self.show.cues {
                    if matches(&cue.color) {
                        cue.color = Some(new.value.clone());
                        recolored += 1;
                    }
                }
                if let Some(defaults) = &mut self.show.pad_defaults
                    && matches(&defaults.color)
                {
                    defaults.color = Some(new.value.clone());
                }
            }
        }
        if self.show.colors != colors || recolored > 0 {
            self.show.colors = colors;
            self.dirty = true;
        }
        recolored
    }

    /// Settings for new pads (`None` = the built-in defaults).
    pub fn set_pad_defaults(&mut self, defaults: Option<PadDefaults>) {
        let defaults = defaults.filter(|d| *d != PadDefaults::default());
        if self.show.pad_defaults != defaults {
            self.show.pad_defaults = defaults;
            self.dirty = true;
        }
    }

    /// A page's own settings for new pads (`None` = use the show's).
    pub fn set_bank_defaults(&mut self, bank: u32, defaults: Option<PadDefaults>) -> Result<()> {
        if bank as usize >= self.show.banks.len() {
            return Err(SessionError::Show(ShowError::Invalid(format!(
                "there is no page {}",
                bank + 1
            ))));
        }
        let before = self.show.bank_defaults.clone();
        self.show.bank_defaults.retain(|d| d.bank != bank);
        if let Some(defaults) = defaults {
            self.show
                .bank_defaults
                .push(BankDefaults { bank, defaults });
            self.show.bank_defaults.sort_by_key(|d| d.bank);
        }
        if self.show.bank_defaults != before {
            self.dirty = true;
        }
        Ok(())
    }

    /// The settings new pads on `bank` get: the page's own, else the
    /// show's, else the built-in ones.
    pub fn effective_pad_defaults(&self, bank: u32) -> PadDefaults {
        self.show
            .bank_defaults
            .iter()
            .find(|d| d.bank == bank)
            .map(|d| d.defaults.clone())
            .or_else(|| self.show.pad_defaults.clone())
            .unwrap_or_default()
    }

    /// Gives existing pads their page's settings for new pads (colour,
    /// fades, loop, group, output, ducking, level matching): every pad, or
    /// with `bank` only that page's. Returns how many pads changed.
    pub fn apply_pad_defaults(&mut self, bank: Option<u32>) -> Result<usize> {
        let slots = self.show.slots.max(1);
        let outputs = self.show.outputs.len();
        let mut changed = Vec::new();
        for cue in &self.show.cues {
            let cue_bank = cue.slot / slots;
            if bank.is_some_and(|b| b != cue_bank) {
                continue;
            }
            let mut updated = cue.clone();
            if self
                .effective_pad_defaults(cue_bank)
                .apply_to(&mut updated, outputs)
            {
                changed.push(updated);
            }
        }
        let count = changed.len();
        for cue in changed {
            self.set_cue(cue)?;
        }
        Ok(count)
    }
}
