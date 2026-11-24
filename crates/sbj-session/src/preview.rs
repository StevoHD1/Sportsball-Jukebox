//! Previewing (auditioning) a pad on a separate output, such as the
//! operator's headphones, without affecting what the crowd hears: no
//! exclusive groups, no ducking, and the pad's own players untouched.
//!
//! Each output's engine has one extra player, after the two per pad,
//! reserved for previews.

use sbj_backend::AudioBackend;
use sbj_engine::{Clip, PlayerId, Timeline, samples_to_seconds};

use crate::show::MAX_SLOTS;
use crate::{Result, Session, SessionError, ShowError, show};

/// The player reserved for previews on every output.
pub(crate) const PREVIEW_PLAYER: PlayerId = PlayerId(MAX_SLOTS * 2);

/// What is being previewed, for display.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PreviewState {
    pub slot: u32,
    pub playing: bool,
    pub position_seconds: f64,
}

impl<B: AudioBackend> Session<B> {
    /// Chooses the output used for previews (`None` turns preview off).
    pub fn set_preview_output(&mut self, output: Option<u32>) -> Result<()> {
        if let Some(o) = output
            && o as usize >= self.show.outputs.len()
        {
            return Err(SessionError::InvalidOutput(o));
        }
        self.stop_preview();
        if self.show.preview_output != output {
            self.show.preview_output = output;
            self.dirty = true;
        }
        Ok(())
    }

    /// Plays a pad on the preview output from the start. Previewing the
    /// pad already being previewed stops it instead. Playlist pads
    /// preview their next song.
    pub fn preview_cue(&mut self, slot: u32) -> Result<()> {
        let cue = self
            .show
            .cue(slot)
            .cloned()
            .ok_or(SessionError::InvalidCue(slot))?;
        let output = self.show.preview_output.ok_or_else(|| {
            SessionError::Show(ShowError::Invalid(
                "no preview output is set (choose one in Audio Outputs)".into(),
            ))
        })? as usize;
        if self.previewing.map(|(s, _)| s) == Some(slot)
            && self.preview_state().is_some_and(|p| p.playing)
        {
            self.stop_preview();
            return Ok(());
        }
        self.stop_preview();
        let rate = self
            .outputs
            .get(output)
            .and_then(|o| o.live.as_ref())
            .map(|l| l.config.sample_rate)
            .ok_or(SessionError::NoOutput)?;

        let timeline = match &cue.playlist {
            Some(playlist) => {
                let next = self
                    .playlists
                    .get(&slot)
                    .and_then(|p| p.peek_next())
                    .unwrap_or(0);
                let Some(buffer) = playlist
                    .items
                    .get(next)
                    .and_then(|file| self.library.get_mut(file))
                    .and_then(|sound| sound.at_rate(rate).ok())
                else {
                    return Ok(());
                };
                Timeline::builder(rate)
                    .clip(Clip::new(buffer))
                    .build()
                    .map_err(|e| SessionError::Show(ShowError::Invalid(e.to_string())))?
            }
            None => {
                let audio = self.audio_at_rate(&cue, rate);
                show::cue_timeline(&cue, rate, &audio).0
            }
        };

        let live = self.outputs[output]
            .live
            .as_mut()
            .ok_or(SessionError::NoOutput)?;
        live.controller.load(PREVIEW_PLAYER, timeline)?;
        live.controller.set_route(PREVIEW_PLAYER, 0, 0)?;
        live.controller.set_player_gain(PREVIEW_PLAYER, cue.gain)?;
        live.controller.play(PREVIEW_PLAYER)?;
        self.previewing = Some((slot, output));
        Ok(())
    }

    pub fn stop_preview(&mut self) {
        if let Some((_, output)) = self.previewing.take()
            && let Some(live) = self.outputs.get_mut(output).and_then(|o| o.live.as_mut())
        {
            let _ = live.controller.stop(PREVIEW_PLAYER);
        }
    }

    pub fn preview_state(&self) -> Option<PreviewState> {
        let (slot, output) = self.previewing?;
        let live = self.outputs.get(output)?.live.as_ref()?;
        Some(PreviewState {
            slot,
            playing: live.controller.is_playing(PREVIEW_PLAYER),
            position_seconds: samples_to_seconds(
                live.controller.position(PREVIEW_PLAYER),
                live.config.sample_rate,
            ),
        })
    }
}
