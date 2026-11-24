//! Playlist pads: a list of songs played one per press, or back to back
//! with crossfades.
//!
//! Each pad slot has two engine players: the slot's own (`A`) and a twin
//! at `slot + MAX_SLOTS` (`B`). A playlist alternates between them so the
//! next song can fade in while the last fades out.

use std::time::{SystemTime, UNIX_EPOCH};

use sbj_backend::AudioBackend;
use sbj_engine::{Clip, PlayerId, Timeline, samples_to_seconds, seconds_to_samples};

use crate::show::{CueDoc, MAX_SLOTS, PlaylistDoc, Retrigger};

/// Auto-advance starts the next song this much earlier than the
/// crossfade strictly needs, so the UI's poll interval (~33 ms) can never
/// leave a gap. The crossfade then lasts exactly the time remaining.
const ADVANCE_MARGIN_SECONDS: f64 = 0.05;
use crate::{CueProblem, Result, Session, SessionError};

/// The slot's second player.
pub(crate) fn player_b(slot: u32) -> PlayerId {
    PlayerId(slot + MAX_SLOTS)
}

#[derive(Debug, Clone)]
pub(crate) struct PlaylistState {
    /// Item indices in play order.
    order: Vec<usize>,
    /// Position in `order` of the next item to play.
    next: usize,
    /// Item playing (or last played).
    pub(crate) current: Option<usize>,
    /// The active player is `B`.
    pub(crate) on_b: bool,
    pub(crate) item_seconds: f64,
    /// Auto-advance has already started the next item.
    advanced: bool,
    rng: u64,
}

impl PlaylistState {
    pub(crate) fn new(playlist: &PlaylistDoc, playable: &[bool]) -> Self {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0x9E37_79B9, |d| d.as_nanos() as u64)
            | 1;
        let mut state = Self {
            order: (0..playlist.items.len()).filter(|i| playable[*i]).collect(),
            next: 0,
            current: None,
            on_b: false,
            item_seconds: 0.0,
            advanced: false,
            rng: seed,
        };
        if playlist.shuffle {
            state.shuffle(None);
        }
        state
    }

    fn random(&mut self) -> u64 {
        // xorshift64
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    /// Fisher-Yates, avoiding `avoid` as the first item (so a reshuffle
    /// never plays the same song twice in a row).
    fn shuffle(&mut self, avoid: Option<usize>) {
        for i in (1..self.order.len()).rev() {
            let j = (self.random() % (i as u64 + 1)) as usize;
            self.order.swap(i, j);
        }
        if self.order.len() > 1 && self.order.first().copied() == avoid {
            let last = self.order.len() - 1;
            self.order.swap(0, last);
        }
    }

    /// Takes the next item. At the end of the list it wraps, reshuffling
    /// straight away if shuffled, so the item after this one is always
    /// known (for "up next" displays).
    fn take_next(&mut self, shuffle: bool) -> Option<usize> {
        if self.order.is_empty() {
            return None;
        }
        let item = self.order[self.next.min(self.order.len() - 1)];
        self.next += 1;
        if self.next >= self.order.len() {
            self.next = 0;
            if shuffle {
                self.shuffle(Some(item));
            }
        }
        Some(item)
    }

    /// The item the next press (or auto-advance) will play.
    pub(crate) fn peek_next(&self) -> Option<usize> {
        self.order.get(self.next).copied()
    }

    pub(crate) fn active(&self, slot: u32) -> PlayerId {
        if self.on_b {
            player_b(slot)
        } else {
            PlayerId(slot)
        }
    }
}

impl<B: AudioBackend> Session<B> {
    /// Sets up a playlist pad when it is loaded: checks its files and
    /// resets its position.
    pub(crate) fn load_playlist(&mut self, slot: u32, cue: &CueDoc) {
        let Some(playlist) = &cue.playlist else {
            self.playlists.remove(&slot);
            return;
        };
        let playable: Vec<bool> = playlist
            .items
            .iter()
            .map(|file| {
                let ok = self.library.contains_key(file);
                if !ok {
                    let why = self
                        .file_errors
                        .get(file)
                        .map_or("not loaded".to_string(), Clone::clone);
                    self.problems.push(CueProblem {
                        slot,
                        message: format!("{file}: {why}"),
                    });
                }
                ok
            })
            .collect();
        if playlist.items.is_empty() {
            self.problems.push(CueProblem {
                slot,
                message: "the playlist is empty".into(),
            });
        }
        self.playlists
            .insert(slot, PlaylistState::new(playlist, &playable));
    }

    /// Fires a playlist pad. Returns whether a song started (rather than
    /// the pad being stopped).
    pub(crate) fn fire_playlist(&mut self, slot: u32, cue: &CueDoc) -> Result<bool> {
        let playlist = cue
            .playlist
            .clone()
            .unwrap_or_else(|| PlaylistDoc::new(Vec::new()));
        let playing = self.cue_state(slot)?.playing;
        if playing {
            return match cue.retrigger {
                Retrigger::Restart => self
                    .next_item(slot, cue, playlist.crossfade, true)
                    .map(|()| true),
                Retrigger::Stop => self.stop_cue(slot, 0.0).map(|()| false),
                Retrigger::Fade => self.stop_cue(slot, cue.fade_out_seconds()).map(|()| false),
            };
        }
        self.next_item(slot, cue, cue.fade_in, false).map(|()| true)
    }

    /// Skips a playing playlist pad to its next item (crossfading), or
    /// starts it if stopped. Does nothing for other pads.
    pub fn skip_cue(&mut self, slot: u32) -> Result<()> {
        let Some(cue) = self.show.cue(slot).cloned() else {
            return Err(SessionError::InvalidCue(slot));
        };
        let Some(playlist) = &cue.playlist else {
            return Ok(());
        };
        let playing = self.cue_state(slot)?.playing;
        let fade = if playing {
            playlist.crossfade
        } else {
            cue.fade_in
        };
        self.next_item(slot, &cue, fade, playing)
    }

    /// Starts the next item on the idle player, fading it in over
    /// `fade_in` seconds, and fades out what was playing over the same
    /// time if `crossfade` is set.
    fn next_item(&mut self, slot: u32, cue: &CueDoc, fade_in: f64, crossfade: bool) -> Result<()> {
        let playlist = cue
            .playlist
            .as_ref()
            .ok_or(SessionError::InvalidCue(slot))?;
        let duck = self.duck_applied.get(&slot).copied().unwrap_or(1.0);
        let (route_first, route_count) = self.route_for(slot, cue);
        let Some(state) = self.playlists.get_mut(&slot) else {
            return Ok(());
        };
        let Some(item) = state.take_next(playlist.shuffle) else {
            return Ok(());
        };
        let old = state.active(slot);
        let was_on_b = state.on_b;
        let file = playlist.items[item].clone();

        let (live, _) = self.live_for(slot)?;
        let rate = live.config.sample_rate;
        let Some(buffer) = self
            .library
            .get_mut(&file)
            .and_then(|sound| sound.at_rate(rate).ok())
        else {
            return Ok(());
        };
        let timeline = Timeline::builder(rate)
            .clip(Clip::new(buffer))
            .build()
            .map_err(|e| SessionError::Show(crate::ShowError::Invalid(e.to_string())))?;
        let seconds = samples_to_seconds(timeline.length(), rate);

        let new = if was_on_b {
            PlayerId(slot)
        } else {
            player_b(slot)
        };
        // `live_for` borrowed self; re-borrow the output now the library
        // lookup is done.
        let (live, _) = self.live_for(slot)?;
        let fade = seconds_to_samples(fade_in, rate) as u32;
        live.controller.load(new, timeline)?;
        live.controller.set_route(new, route_first, route_count)?;
        live.controller.set_player_gain_over(
            new,
            if fade > 0 { 0.0 } else { cue.gain * duck },
            0,
        )?;
        live.controller.play(new)?;
        if fade > 0 {
            live.controller
                .set_player_gain_over(new, cue.gain * duck, fade)?;
        }
        if crossfade {
            live.controller
                .fade_out_and_stop(old, u64::from(fade.max(1)))?;
        } else {
            live.controller.stop(old)?;
        }

        if let Some(state) = self.playlists.get_mut(&slot) {
            state.on_b = !was_on_b;
            state.current = Some(item);
            state.item_seconds = seconds;
            state.advanced = false;
        }
        Ok(())
    }

    /// Auto-advance: start the next item when the current one is within
    /// its crossfade of the end. Called on every poll.
    pub(crate) fn advance_playlists(&mut self) {
        let due: Vec<(u32, CueDoc, f64)> = self
            .playlists
            .iter()
            .filter(|(_, state)| !state.advanced && state.current.is_some())
            .filter_map(|(slot, state)| {
                let cue = self.show.cue(*slot)?;
                let playlist = cue.playlist.as_ref()?;
                if !playlist.auto_advance {
                    return None;
                }
                let st = self.cue_state(*slot).ok()?;
                let remaining = state.item_seconds - st.position_seconds;
                let threshold = playlist.crossfade.max(0.0) + ADVANCE_MARGIN_SECONDS;
                (st.playing && remaining <= threshold).then(|| (*slot, cue.clone(), remaining))
            })
            .collect();
        for (slot, cue, remaining) in due {
            if let Some(state) = self.playlists.get_mut(&slot) {
                state.advanced = true;
            }
            // Fade over exactly what is left, so the outgoing song is not
            // cut short.
            let _ = self.next_item(slot, &cue, remaining.max(0.005), true);
            // `next_item` resets `advanced` for the new item.
        }
    }

    /// The cue's output channel route (0-based first, count; 0 = all).
    pub(crate) fn route_for(&self, slot: u32, cue: &CueDoc) -> (u16, u16) {
        let channels = self
            .loaded
            .get(&slot)
            .and_then(|l| self.outputs[l.output].live.as_ref())
            .map_or(0, |live| live.config.channels);
        match cue.route {
            Some(r) if r.first >= 1 && r.count >= 1 && r.first - 1 + r.count <= channels => {
                (r.first - 1, r.count)
            }
            _ => (0, 0),
        }
    }
}
