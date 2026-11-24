//! What happens when a pad is fired: vamps, retrigger behaviour,
//! exclusive groups and ducking.

use std::time::{Duration, Instant};

use sbj_backend::AudioBackend;
use sbj_engine::{Event, PlayerId, seconds_to_samples};

use crate::show::{ActionKind, PadTarget, Retrigger};
use crate::{CueDoc, DEFAULT_FADE_OUT, Result, Session};

/// A fired cue counts as playing for this long even before the audio
/// thread reports it (it starts at the next audio block).
const START_GRACE: Duration = Duration::from_millis(250);
/// How fast other cues dip when a ducking cue starts, and recover after.
const DUCK_ATTACK_SECONDS: f64 = 0.15;
const DUCK_RELEASE_SECONDS: f64 = 0.8;

impl<B: AudioBackend> Session<B> {
    pub(crate) fn fire(&mut self, slot: u32) -> Result<()> {
        let Some(cue) = self.show.cue(slot).cloned() else {
            return Ok(());
        };
        if cue.playlist.is_some() {
            if self.fire_playlist(slot, &cue)? {
                self.after_start(slot, &cue);
            }
            return Ok(());
        }
        let player = PlayerId(slot);
        let duck = self.duck_applied.get(&slot).copied().unwrap_or(1.0);
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
                    let fade = seconds_to_samples(cue.fade_out_seconds(), live.config.sample_rate);
                    live.controller.fade_out_and_stop(player, fade)?;
                    self.recent_fires.remove(&slot);
                    return Ok(());
                }
            }
        }
        let fade_in = seconds_to_samples(cue.fade_in, live.config.sample_rate) as u32;
        if fade_in > 0 {
            live.controller.set_player_gain_over(player, 0.0, 0)?;
        }
        live.controller.restart(player)?;
        if fade_in > 0 {
            live.controller
                .set_player_gain_over(player, cue.gain * duck, fade_in)?;
        }
        self.log_start(slot, None);
        self.after_start(slot, &cue);
        Ok(())
    }

    /// After a pad starts: exclusive-group hand-over and ducking.
    fn after_start(&mut self, slot: u32, cue: &crate::CueDoc) {
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
    }

    /// Follows up markers that act on other pads (called from `poll` with
    /// the engine events). Pads on the same output were already started
    /// or stopped by the engine at the marker's exact sample; here they
    /// get what firing a pad also does (log entry, group hand-over,
    /// ducking). Pads on other outputs, and playlists, are fired or
    /// stopped now. Logged with the source "marker".
    pub(crate) fn handle_marker_events(&mut self, events: &[Event]) {
        for event in events {
            match *event {
                Event::Triggered { player, .. } => {
                    let slot = player.0;
                    let Some(cue) = self.show.cue(slot).cloned() else {
                        continue;
                    };
                    self.log.source = "marker";
                    self.log_start(slot, None);
                    self.after_start(slot, &cue);
                }
                Event::MarkerReached { player, marker, .. } => {
                    let Some(source) = self.show.cue(player.0) else {
                        continue;
                    };
                    let Some(doc) = source.markers.iter().find(|m| m.id == marker.0) else {
                        continue;
                    };
                    let (action, name) = (doc.action, doc.name.clone());
                    let Some(pad) = action.target_pad() else {
                        continue;
                    };
                    if self.pad_target(source, pad) != PadTarget::Elsewhere {
                        continue; // done by the engine
                    }
                    self.log.source = "marker";
                    let result = if action.kind == ActionKind::Fire {
                        self.fire_cue(pad)
                    } else {
                        let fade = action.seconds.unwrap_or_else(|| {
                            self.show
                                .cue(pad)
                                .map_or(DEFAULT_FADE_OUT, CueDoc::fade_out_seconds)
                        });
                        self.stop_cue(pad, fade)
                    };
                    if let Err(e) = result {
                        self.notices
                            .push(format!("Marker \"{name}\" could not act on its pad: {e}"));
                    }
                }
                _ => {}
            }
        }
        self.log.source = "operator";
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
                // Both of the slot's players (playlists alternate them).
                let gain = cue.gain * target;
                let ok = live
                    .controller
                    .set_player_gain_over(PlayerId(slot), gain, frames)
                    .and_then(|()| {
                        live.controller.set_player_gain_over(
                            crate::playlist::player_b(slot),
                            gain,
                            frames,
                        )
                    })
                    .is_ok();
                if ok {
                    self.duck_applied.insert(slot, target);
                }
            }
        }
    }
}
