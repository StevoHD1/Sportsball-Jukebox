//! Undo / redo of show edits.
//!
//! The app marks each user action with [`Session::record_undo`]; if the
//! show differs from the last recorded state, that state is pushed with
//! the action's name. Undo and redo swap whole show documents, then
//! reload only the pads whose settings actually changed, so pads that are
//! playing and weren't touched keep playing.

use std::collections::BTreeSet;

use sbj_backend::AudioBackend;

use crate::{Session, Show};

/// Most undo steps kept.
const UNDO_DEPTH: usize = 50;

/// Undo history: the show as it was before each recorded action.
#[derive(Debug, Default)]
pub(crate) struct UndoHistory {
    /// The show as of the last recorded action (what undo returns to).
    base: Show,
    undo: Vec<(String, Show)>,
    redo: Vec<(String, Show)>,
}

impl<B: AudioBackend> Session<B> {
    /// Records the current show as one undoable step named `label` (e.g.
    /// "Paste Style"), if anything changed since the last one. Returns
    /// whether a step was recorded.
    pub fn record_undo(&mut self, label: &str) -> bool {
        if self.show == self.history.base {
            return false;
        }
        let before = std::mem::replace(&mut self.history.base, self.show.clone());
        self.history.undo.push((label.to_string(), before));
        if self.history.undo.len() > UNDO_DEPTH {
            self.history.undo.remove(0);
        }
        self.history.redo.clear();
        true
    }

    /// Forgets the history (a show was opened or created).
    pub(crate) fn reset_undo(&mut self) {
        self.history = UndoHistory {
            base: self.show.clone(),
            ..UndoHistory::default()
        };
    }

    /// The name of the step Undo would undo.
    pub fn undo_label(&self) -> Option<&str> {
        self.history.undo.last().map(|(label, _)| label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.history.redo.last().map(|(label, _)| label.as_str())
    }

    /// Undoes the last recorded step (anything changed since it is
    /// recorded first, so it isn't lost). Returns its name.
    pub fn undo(&mut self) -> Option<String> {
        self.record_undo("Change");
        let (label, before) = self.history.undo.pop()?;
        let after = std::mem::replace(&mut self.history.base, before.clone());
        self.history.redo.push((label.clone(), after));
        self.restore_show(before);
        Some(label)
    }

    pub fn redo(&mut self) -> Option<String> {
        let (label, after) = self.history.redo.pop()?;
        let before = std::mem::replace(&mut self.history.base, after.clone());
        self.history.undo.push((label.clone(), before));
        self.restore_show(after);
        Some(label)
    }

    /// Puts `target` in place of the show, reloading only what changed.
    fn restore_show(&mut self, target: Show) {
        let old = std::mem::replace(&mut self.show, target);
        if old.outputs != self.show.outputs {
            // Reopen outputs as for an edit (this reloads their pads).
            let outputs = self.show.outputs.clone();
            self.show.outputs = old.outputs.clone();
            let _ = self.set_outputs(outputs);
        }
        let everything = old.slots != self.show.slots
            || old.level_match != self.show.level_match
            || old.outputs != self.show.outputs;
        let mut slots: BTreeSet<u32> = BTreeSet::new();
        for cue in old.cues.iter().chain(&self.show.cues) {
            if everything || old.cue(cue.slot) != self.show.cue(cue.slot) {
                slots.insert(cue.slot);
            }
        }
        // Pads whose markers reach a changed pad bake that in.
        let referrers: Vec<u32> = self
            .show
            .cues
            .iter()
            .filter(|c| {
                c.markers
                    .iter()
                    .any(|m| m.action.target_pad().is_some_and(|p| slots.contains(&p)))
            })
            .map(|c| c.slot)
            .collect();
        slots.extend(referrers);
        for slot in slots {
            self.load_cue(slot);
        }
        let banks = self.show.banks.len().max(1) as u32;
        self.current_bank = self.current_bank.min(banks - 1);
        if self.standby.is_some_and(|i| i >= self.show.rundown.len()) {
            self.standby = self.pad_step_from(0);
        }
        self.dirty = true;
    }
}
