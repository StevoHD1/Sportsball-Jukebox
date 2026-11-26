//! Live transport helpers: scrubbing a playing pad, "fade/stop the last
//! pad fired", and single-play banks (Multiplay off).

use sbj_backend::AudioBackend;
use sbj_engine::{PlayerId, seconds_to_samples};

use crate::{Result, Session};

/// How many recent fires are remembered for Fade Last / Stop Last.
const FIRE_HISTORY: usize = 64;

impl<B: AudioBackend> Session<B> {
    /// Moves a playing pad's playhead to `seconds` (clamped to its length),
    /// with the usual click-free crossfade. Does nothing if it isn't
    /// playing. Playlist pads move within the current song.
    pub fn seek_cue(&mut self, slot: u32, seconds: f64) -> Result<()> {
        let player = self
            .playlists
            .get(&slot)
            .map_or(PlayerId(slot), |p| p.active(slot));
        let item_seconds = self.playlists.get(&slot).map(|p| p.item_seconds);
        let (live, loaded) = self.live_for(slot)?;
        if !live.controller.is_playing(player) {
            return Ok(());
        }
        let length = item_seconds.unwrap_or(loaded.length_seconds);
        let seconds = if seconds.is_finite() { seconds } else { 0.0 };
        let position =
            seconds_to_samples(seconds.clamp(0.0, length.max(0.0)), live.config.sample_rate);
        live.controller.seek(player, position)?;
        Ok(())
    }

    /// Notes that a pad started (for Fade Last / Stop Last).
    pub(crate) fn remember_fire(&mut self, slot: u32) {
        self.fire_order.retain(|&s| s != slot);
        self.fire_order.push(slot);
        if self.fire_order.len() > FIRE_HISTORY {
            self.fire_order.remove(0);
        }
    }

    /// The most recently fired pad that is still playing.
    pub fn last_playing(&self) -> Option<u32> {
        self.fire_order
            .iter()
            .rev()
            .copied()
            .find(|&slot| self.is_active(slot))
    }

    /// Fades out (over the pad's own fade-out) or stops the most recently
    /// fired pad that is still playing. Returns which pad.
    pub fn stop_last(&mut self, fade: bool) -> Result<Option<u32>> {
        let Some(slot) = self.last_playing() else {
            return Ok(None);
        };
        let seconds = if fade {
            self.show
                .cue(slot)
                .map_or(crate::DEFAULT_FADE_OUT, crate::CueDoc::fade_out_seconds)
        } else {
            0.0
        };
        self.stop_cue(slot, seconds)?;
        self.fire_order.retain(|&s| s != slot);
        Ok(Some(slot))
    }

    /// Multiplay on (the default): pads in the bank play over each other.
    /// Off (single-play): firing a pad fades out the bank's other pads.
    pub fn set_bank_multiplay(&mut self, bank: u32, multiplay: bool) {
        if bank as usize >= self.show.banks.len() {
            return;
        }
        let single = self.show.single_play.contains(&bank);
        if single == multiplay {
            if multiplay {
                self.show.single_play.retain(|&b| b != bank);
            } else {
                self.show.single_play.push(bank);
                self.show.single_play.sort_unstable();
            }
            self.dirty = true;
        }
    }

    pub fn bank_multiplay(&self, bank: u32) -> bool {
        !self.show.single_play.contains(&bank)
    }

    /// Single-play (a solo pad, or a pad in a bank with Multiplay off):
    /// fade out the bank's other playing pads when one is fired, over the
    /// show's group hand-over time. Pads set to play over everything are
    /// left alone.
    pub(crate) fn single_play_handover(&mut self, slot: u32) {
        if !self.plays_solo(slot) {
            return;
        }
        let slots = self.show.slots.max(1);
        let bank = slot / slots;
        let others: Vec<u32> = self
            .show
            .cues
            .iter()
            .map(|c| c.slot)
            .filter(|&s| s != slot && s / slots == bank && !self.plays_over(s))
            .collect();
        let fade = self.show.group_fade;
        for other in others {
            if self.is_active(other) {
                let _ = self.stop_cue(other, fade);
            }
        }
    }
}
