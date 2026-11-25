//! The game rundown: an ordered list of pads (and notes) fired one at a
//! time with GO, from the app, the space bar, MIDI, OSC or the remote.
//!
//! One pad step is on **standby**; GO fires it (exactly as pressing the
//! pad does) and moves standby to the next pad step. Notes are never on
//! standby: they are headings and reminders between steps.

use sbj_backend::AudioBackend;

use crate::show::RundownItem;
use crate::{Result, Session, SessionError, ShowError};

impl<B: AudioBackend> Session<B> {
    pub fn rundown(&self) -> &[RundownItem] {
        &self.show.rundown
    }

    /// Index of the standby step, or `None` when the rundown is finished
    /// (or has no pad steps).
    pub fn standby(&self) -> Option<usize> {
        self.standby
    }

    /// Replaces the rundown. Standby stays on the same step if it still
    /// exists, else moves to the first pad step.
    pub fn set_rundown(&mut self, items: Vec<RundownItem>) -> Result<()> {
        let mut ids = std::collections::HashSet::new();
        if let Some(dup) = items.iter().find(|i| !ids.insert(i.id)) {
            return Err(SessionError::Show(ShowError::Invalid(format!(
                "two rundown steps with id {}",
                dup.id
            ))));
        }
        let current = self
            .standby
            .and_then(|i| self.show.rundown.get(i))
            .map(|item| item.id);
        self.show.rundown = items;
        self.dirty = true;
        self.standby =
            match current.and_then(|id| self.show.rundown.iter().position(|i| i.id == id)) {
                Some(i) => self.pad_step_from(i),
                None => self.pad_step_from(0),
            };
        Ok(())
    }

    /// Puts step `index` on standby (a note moves on to the next pad
    /// step); `None` clears standby.
    pub fn set_standby(&mut self, index: Option<usize>) {
        self.standby = index.and_then(|i| self.pad_step_from(i));
    }

    /// Moves standby to the next (or previous) pad step without firing.
    /// Back from "finished" goes to the last pad step.
    pub fn step_standby(&mut self, forward: bool) {
        let len = self.show.rundown.len();
        self.standby = match (self.standby, forward) {
            (Some(i), true) => self.pad_step_from(i + 1),
            (Some(i), false) => self.pad_step_before(i).or(Some(i)),
            (None, true) => None,
            (None, false) => self.pad_step_before(len),
        };
    }

    /// Fires the standby step and moves standby on. Does nothing when the
    /// rundown is finished. If the pad cannot fire (e.g. its device is
    /// unplugged), standby stays put so GO can be pressed again. Returns
    /// the slot fired.
    pub fn rundown_go(&mut self) -> Result<Option<u32>> {
        let Some(index) = self.standby else {
            return Ok(None);
        };
        let Some(pad) = self.show.rundown.get(index).and_then(|i| i.pad) else {
            self.standby = self.pad_step_from(index);
            return Ok(None);
        };
        if self.log.source == "operator" {
            self.log.source = "rundown";
        }
        let result = self.fire_cue(pad);
        if self.log.source == "rundown" {
            self.log.source = "operator";
        }
        result?;
        self.standby = self.pad_step_from(index + 1);
        Ok(Some(pad))
    }

    /// Title of a step: its text, else its pad's title.
    pub fn rundown_label(&self, index: usize) -> Option<String> {
        let item = self.show.rundown.get(index)?;
        if !item.text.is_empty() {
            return Some(item.text.clone());
        }
        let pad = item.pad?;
        Some(match self.show.cue(pad) {
            Some(cue) if !cue.title.is_empty() => cue.title.clone(),
            Some(_) | None => {
                let slots = self.show.slots.max(1);
                format!("Bank {} pad {}", pad / slots + 1, pad % slots + 1)
            }
        })
    }

    /// The first pad step at or after `from`.
    pub(crate) fn pad_step_from(&self, from: usize) -> Option<usize> {
        self.show
            .rundown
            .iter()
            .enumerate()
            .skip(from)
            .find(|(_, item)| item.pad.is_some())
            .map(|(i, _)| i)
    }

    /// The last pad step before `before`.
    fn pad_step_before(&self, before: usize) -> Option<usize> {
        self.show.rundown[..before.min(self.show.rundown.len())]
            .iter()
            .rposition(|item| item.pad.is_some())
    }
}
