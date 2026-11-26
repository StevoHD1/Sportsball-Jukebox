//! Bulk edits: change chosen settings on every pad in the show or on one
//! bank ("fade out 2 s on every pad in this bank"). Only the settings
//! given change; everything else on each pad stays as it is.

use sbj_backend::AudioBackend;
use serde::{Deserialize, Deserializer, Serialize};

use crate::show::{PlayMode, Retrigger};
use crate::{Result, Session, SessionError};

/// Tells a missing field (leave alone) from `null` (clear it).
fn present<'de, D, T>(d: D) -> std::result::Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Some(Option::deserialize(d)?))
}

/// The settings to change. A field left out (`None`) is left alone; for
/// the clearable ones, `Some(None)` (JSON `null`) clears it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PadChanges {
    /// Colour name or "#RRGGBB"; null = the default colour.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub color: Option<Option<String>>,
    /// Linear gain (1 = as recorded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gain: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_in: Option<f64>,
    /// Seconds; null = the default fade-out.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub fade_out: Option<Option<f64>>,
    #[serde(default, rename = "loop", skip_serializing_if = "Option::is_none")]
    pub looping: Option<bool>,
    /// Exclusive group; null = none.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub group: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<u32>,
    /// Duck other pads by this many dB; null = don't duck.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub duck: Option<Option<f32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level_match: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retrigger: Option<Retrigger>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub play_mode: Option<PlayMode>,
}

impl PadChanges {
    /// Applies the changes to one pad; returns whether it changed.
    pub fn apply_to(&self, cue: &mut crate::CueDoc) -> bool {
        let before = cue.clone();
        if let Some(color) = &self.color {
            cue.color.clone_from(color);
        }
        if let Some(gain) = self.gain {
            cue.gain = gain.max(0.0);
        }
        if let Some(fade_in) = self.fade_in {
            cue.fade_in = fade_in.max(0.0);
        }
        if let Some(fade_out) = self.fade_out {
            cue.fade_out = fade_out.map(|f| f.max(0.0));
        }
        if let Some(looping) = self.looping {
            cue.looping = looping;
        }
        if let Some(group) = &self.group {
            cue.group = group
                .as_ref()
                .map(|g| g.trim().to_string())
                .filter(|g| !g.is_empty());
        }
        if let Some(output) = self.output {
            cue.output = output;
        }
        if let Some(duck) = self.duck {
            cue.duck = duck;
        }
        if let Some(level_match) = self.level_match {
            cue.level_match = level_match;
        }
        if let Some(retrigger) = self.retrigger {
            cue.retrigger = retrigger;
        }
        if let Some(play_mode) = self.play_mode {
            cue.play_mode = play_mode;
        }
        *cue != before
    }
}

impl<B: AudioBackend> Session<B> {
    /// Changes the given settings on every pad, or with `bank` on that
    /// bank's pads. Returns how many pads changed.
    pub fn bulk_edit(&mut self, bank: Option<u32>, changes: &PadChanges) -> Result<usize> {
        if let Some(output) = changes.output
            && output as usize >= self.show.outputs.len()
        {
            return Err(SessionError::InvalidOutput(output));
        }
        let slots = self.show.slots.max(1);
        let mut changed = Vec::new();
        for cue in &self.show.cues {
            if bank.is_some_and(|b| b != cue.slot / slots) {
                continue;
            }
            let mut updated = cue.clone();
            if changes.apply_to(&mut updated) {
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
