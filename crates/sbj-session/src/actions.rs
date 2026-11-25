//! Pad actions: what a pad does, besides playing, each time it starts —
//! stop other pads, fire or stop a pad (optionally after a delay), duck
//! everything else for a while, switch bank — and its play mode
//! (follow the bank's Multiplay, play over, or solo).

use std::time::{Duration, Instant};

use sbj_backend::AudioBackend;

use crate::show::{FireActionKind, PlayMode};
use crate::{CueDoc, DEFAULT_FADE_OUT, Session};

/// Pad actions nested deeper than this (A fires B fires C …) are skipped,
/// so pads that fire each other can't loop forever.
const MAX_ACTION_DEPTH: u8 = 4;

/// "Duck others for X seconds" in effect.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TimedDuck {
    pub source: u32,
    pub db: f32,
    pub until: Instant,
}

impl<B: AudioBackend> Session<B> {
    /// Runs the pad's actions, in order. Called whenever it starts, however
    /// it was fired. Plays started by actions are logged as "action".
    pub(crate) fn run_fire_actions(&mut self, slot: u32) {
        let Some(cue) = self.show.cue(slot).cloned() else {
            return;
        };
        if cue.on_fire.is_empty() || self.action_depth >= MAX_ACTION_DEPTH {
            return;
        }
        self.action_depth += 1;
        let source = self.log.source;
        for action in &cue.on_fire {
            match action.kind {
                FireActionKind::StopOthers => {
                    let fade = action.seconds.unwrap_or(self.show.stop_all_fade).max(0.0);
                    let slots = self.show.slots.max(1);
                    let others: Vec<u32> = self
                        .show
                        .cues
                        .iter()
                        .map(|c| c.slot)
                        .filter(|&s| s != slot && (!action.this_bank || s / slots == slot / slots))
                        .collect();
                    for other in others {
                        if self.is_active(other) {
                            let _ = self.stop_cue(other, fade);
                        }
                    }
                }
                FireActionKind::FirePad => {
                    let Some(pad) = action.pad else { continue };
                    let delay = action.seconds.unwrap_or(0.0);
                    if delay > 0.0 && delay.is_finite() {
                        let at = Instant::now() + Duration::from_secs_f64(delay.min(3600.0));
                        self.scheduled_fires.push((at, pad));
                    } else {
                        self.log.source = "action";
                        if let Err(e) = self.fire_cue(pad) {
                            self.notices
                                .push(format!("\"{}\" could not fire its pad: {e}", cue.title));
                        }
                        self.log.source = source;
                    }
                }
                FireActionKind::StopPad => {
                    let Some(pad) = action.pad else { continue };
                    let fade = action.seconds.unwrap_or_else(|| {
                        self.show
                            .cue(pad)
                            .map_or(DEFAULT_FADE_OUT, CueDoc::fade_out_seconds)
                    });
                    let _ = self.stop_cue(pad, fade.max(0.0));
                }
                FireActionKind::Duck => {
                    let db = action.db.unwrap_or(-12.0);
                    let db = if db.is_finite() {
                        db.clamp(-60.0, 0.0)
                    } else {
                        -12.0
                    };
                    let seconds = action.seconds.unwrap_or(5.0);
                    let seconds = if seconds.is_finite() {
                        seconds.clamp(0.0, 3600.0)
                    } else {
                        5.0
                    };
                    self.timed_ducks.retain(|d| d.source != slot);
                    self.timed_ducks.push(TimedDuck {
                        source: slot,
                        db,
                        until: Instant::now() + Duration::from_secs_f64(seconds),
                    });
                    self.update_ducking();
                }
                FireActionKind::Bank => {
                    if let Some(bank) = action.bank {
                        let _ = self.set_bank(bank);
                    }
                }
            }
        }
        self.action_depth -= 1;
    }

    /// Fires pads whose "after X seconds" has come (called from `poll`).
    pub(crate) fn run_scheduled_fires(&mut self) {
        let now = Instant::now();
        let (due, waiting): (Vec<_>, Vec<_>) = self
            .scheduled_fires
            .drain(..)
            .partition(|(at, _)| *at <= now);
        self.scheduled_fires = waiting;
        for (_, pad) in due {
            let source = self.log.source;
            self.log.source = "action";
            let _ = self.fire_cue(pad);
            self.log.source = source;
        }
    }

    /// How a pad treats Multiplay: its own play mode, else its bank's.
    pub(crate) fn plays_solo(&self, slot: u32) -> bool {
        match self.show.cue(slot).map_or(PlayMode::Bank, |c| c.play_mode) {
            PlayMode::Over => false,
            PlayMode::Solo => true,
            PlayMode::Bank => !self.bank_multiplay(slot / self.show.slots.max(1)),
        }
    }

    /// Pads that play over everything aren't faded out by solo pads.
    pub(crate) fn plays_over(&self, slot: u32) -> bool {
        self.show
            .cue(slot)
            .is_some_and(|c| c.play_mode == PlayMode::Over)
    }

    /// The "duck others" level (dB, 0 = none) timed ducks give `slot`.
    pub(crate) fn timed_duck_db(&self, slot: u32, now: Instant) -> f32 {
        self.timed_ducks
            .iter()
            .filter(|d| d.source != slot && d.until > now)
            .map(|d| d.db)
            .fold(0.0, f32::min)
    }

    /// Stops pads' timed ducks after a Stop All, and drops expired ones.
    pub(crate) fn expire_timed_ducks(&mut self, now: Instant) {
        self.timed_ducks.retain(|d| d.until > now);
    }
}
