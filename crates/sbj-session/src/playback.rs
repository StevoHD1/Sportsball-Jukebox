//! What happens when a pad is fired: vamps, retrigger behaviour,
//! exclusive groups and ducking.

use std::time::{Duration, Instant};

use sbj_backend::AudioBackend;
use sbj_engine::{PlayerId, seconds_to_samples};

use crate::show::Retrigger;
use crate::{Result, Session};

/// A fired cue counts as playing for this long even before the audio
/// thread reports it (it starts at the next audio block).
const START_GRACE: Duration = Duration::from_millis(250);
/// How fast other cues dip when a ducking cue starts, and recover after.
const DUCK_ATTACK_SECONDS: f64 = 0.15;
const DUCK_RELEASE_SECONDS: f64 = 0.8;
/// Retrigger "fade" length.
const RETRIGGER_FADE_SECONDS: f64 = 0.5;

impl<B: AudioBackend> Session<B> {
    pub(crate) fn fire(&mut self, slot: u32) -> Result<()> {
        let Some(cue) = self.show.cue(slot).cloned() else {
            return Ok(());
        };
        let player = PlayerId(slot);
        let (live, _) = self.live_for(slot)?;
        if live.controller.is_playing(player) {
            if live.controller.is_holding(player) {
                // "Go": leave the vamp at its loop point.
                live.controller.release(player)?;
                return Ok(());
            }
            match cue.retrigger {
                Retrigger::Restart => {}
                Retrigger::Stop => {
                    live.controller.stop(player)?;
                    self.recent_fires.remove(&slot);
                    return Ok(());
                }
                Retrigger::Fade => {
                    let fade = seconds_to_samples(RETRIGGER_FADE_SECONDS, live.config.sample_rate);
                    live.controller.fade_out_and_stop(player, fade)?;
                    self.recent_fires.remove(&slot);
                    return Ok(());
                }
            }
        }
        live.controller.restart(player)?;
        self.recent_fires.insert(slot, Instant::now());

        // Exclusive group: hand over from whatever else in the group plays.
        if let Some(group) = cue
            .group
            .as_deref()
            .map(str::trim)
            .filter(|g| !g.is_empty())
        {
            let others: Vec<u32> = self
                .show
                .cues
                .iter()
                .filter(|c| c.slot != slot)
                .filter(|c| {
                    c.group
                        .as_deref()
                        .is_some_and(|g| g.trim().eq_ignore_ascii_case(group))
                })
                .map(|c| c.slot)
                .collect();
            let fade = self.show.group_fade;
            for other in others {
                if self.is_active(other) {
                    let _ = self.stop_cue(other, fade);
                    self.recent_fires.remove(&other);
                }
            }
        }
        self.update_ducking();
        Ok(())
    }

    /// Playing, or fired so recently the audio thread has not started it.
    pub(crate) fn is_active(&self, slot: u32) -> bool {
        let fired = self
            .recent_fires
            .get(&slot)
            .is_some_and(|t| t.elapsed() < START_GRACE);
        fired || self.cue_state(slot).is_ok_and(|s| s.playing)
    }

    /// Applies ducking: on each output, while any cue with a `duck` level
    /// is active, every other cue on that output is lowered by the deepest
    /// such level. Called after firing and on every poll.
    pub(crate) fn update_ducking(&mut self) {
        // A fired cue stops needing the grace period once the audio thread
        // reports it playing (or the grace runs out because it never did).
        let started: Vec<u32> = self
            .recent_fires
            .keys()
            .copied()
            .filter(|slot| self.cue_state(*slot).is_ok_and(|s| s.playing))
            .collect();
        for slot in started {
            self.recent_fires.remove(&slot);
        }
        self.recent_fires
            .retain(|_, fired| fired.elapsed() < START_GRACE);
        let loaded: Vec<(u32, usize)> = self.loaded.iter().map(|(s, l)| (*s, l.output)).collect();

        for output in 0..self.outputs.len() {
            let mut duck_db: f32 = 0.0;
            for &(slot, o) in &loaded {
                if o != output || !self.is_active(slot) {
                    continue;
                }
                if let Some(db) = self.show.cue(slot).and_then(|c| c.duck) {
                    duck_db = duck_db.min(db.min(0.0));
                }
            }
            let factor = 10f32.powf(duck_db / 20.0);

            let Some(live) = self.outputs[output].live.as_mut() else {
                continue;
            };
            let rate = live.config.sample_rate;
            for &(slot, o) in &loaded {
                if o != output {
                    continue;
                }
                let Some(cue) = self.show.cues.iter().find(|c| c.slot == slot) else {
                    continue;
                };
                // A ducking cue is never ducked itself.
                let target = if cue.duck.is_some() { 1.0 } else { factor };
                let current = self.duck_applied.get(&slot).copied().unwrap_or(1.0);
                if (current - target).abs() < 1e-6 {
                    continue;
                }
                let seconds = if target < current {
                    DUCK_ATTACK_SECONDS
                } else {
                    DUCK_RELEASE_SECONDS
                };
                let frames = seconds_to_samples(seconds, rate) as u32;
                if live
                    .controller
                    .set_player_gain_over(PlayerId(slot), cue.gain * target, frames)
                    .is_ok()
                {
                    self.duck_applied.insert(slot, target);
                }
            }
        }
    }
}
