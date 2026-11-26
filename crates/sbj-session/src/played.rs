//! "Played" marks: which pads have been used this game, so the operator
//! sees at a glance what's left. Marks belong to the running session, not
//! the show file (firing a pad shouldn't make the show "edited" or fill
//! the undo history); the show keeps only the played colour and whether
//! firing marks a pad.

use std::collections::BTreeSet;

use sbj_backend::AudioBackend;

use crate::Session;

impl<B: AudioBackend> Session<B> {
    /// Pads marked as played.
    pub fn played(&self) -> &BTreeSet<u32> {
        &self.played
    }

    /// Marks (or unmarks) a pad as played.
    pub fn set_played(&mut self, slot: u32, played: bool) {
        if played && self.show.cue(slot).is_some() {
            self.played.insert(slot);
        } else {
            self.played.remove(&slot);
        }
    }

    /// Unmarks every pad in `bank` (0-based), or every pad if `None`.
    pub fn reset_played(&mut self, bank: Option<u32>) {
        let slots = self.show.slots.max(1);
        match bank {
            Some(bank) => self.played.retain(|&s| s / slots != bank),
            None => self.played.clear(),
        }
    }

    /// The played colour (`None` turns it off) and whether firing a pad
    /// marks it. Saved with the show.
    pub fn set_played_style(&mut self, color: Option<String>, mark_on_fire: bool) {
        let color = color
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty());
        if self.show.played_color != color || self.show.mark_played != mark_on_fire {
            self.show.played_color = color;
            self.show.mark_played = mark_on_fire;
            self.dirty = true;
        }
    }

    /// Called whenever a pad starts.
    pub(crate) fn auto_mark_played(&mut self, slot: u32) {
        if self.show.played_color.is_some() && self.show.mark_played {
            self.played.insert(slot);
        }
    }
}
