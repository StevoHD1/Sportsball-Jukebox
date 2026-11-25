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
/// `PreviewState::slot` while a library file (not a pad) is previewed.
pub const FILE_PREVIEW: u32 = u32::MAX;

/// What is being previewed, for display.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewState {
    /// The pad, or [`FILE_PREVIEW`] for a library file.
    pub slot: u32,
    /// The library file being previewed, if any.
    pub file: Option<String>,
    pub playing: bool,
    pub position_seconds: f64,
    pub duration_seconds: f64,
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
                let gain = self.level_gain(&cue, &playlist.items[next]);
                Timeline::builder(rate)
                    .clip(Clip::new(buffer).with_gain(gain))
                    .build()
                    .map_err(|e| SessionError::Show(ShowError::Invalid(e.to_string())))?
            }
            None => {
                let audio = self.audio_at_rate(&cue, rate);
                // Markers never act on other pads from the preview player.
                show::cue_timeline(
                    &cue,
                    rate,
                    &audio,
                    &|_| show::PadTarget::Elsewhere,
                    &|file| self.level_gain(&cue, file),
                )
                .0
            }
        };

        self.preview_length = samples_to_seconds(timeline.length(), rate);
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

    /// Plays a decoded library file on the preview output, level-matched
    /// like a new pad would be. Previewing it again stops it.
    pub fn preview_file(&mut self, file: &str) -> Result<()> {
        let output = self.show.preview_output.ok_or_else(|| {
            SessionError::Show(ShowError::Invalid(
                "no preview output is set (choose one in Audio Outputs)".into(),
            ))
        })? as usize;
        if self.previewing_file.as_deref() == Some(file)
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
        let gain = self.level_gain(&crate::CueDoc::simple(FILE_PREVIEW, "", file), file);
        let buffer = self
            .library
            .get_mut(file)
            .ok_or_else(|| SessionError::Show(ShowError::Invalid(format!("{file} is not loaded"))))?
            .at_rate(rate)?;
        let timeline = Timeline::builder(rate)
            .clip(Clip::new(buffer).with_gain(gain))
            .build()
            .map_err(|e| SessionError::Show(ShowError::Invalid(e.to_string())))?;
        self.preview_length = samples_to_seconds(timeline.length(), rate);
        let live = self.outputs[output]
            .live
            .as_mut()
            .ok_or(SessionError::NoOutput)?;
        live.controller.load(PREVIEW_PLAYER, timeline)?;
        live.controller.set_route(PREVIEW_PLAYER, 0, 0)?;
        live.controller.set_player_gain(PREVIEW_PLAYER, 1.0)?;
        live.controller.play(PREVIEW_PLAYER)?;
        self.previewing = Some((FILE_PREVIEW, output));
        self.previewing_file = Some(file.to_string());
        Ok(())
    }

    /// Moves the preview's playhead (playing it again if it had stopped),
    /// for the preview player's scrub bar.
    pub fn seek_preview(&mut self, seconds: f64) -> Result<()> {
        let Some((_, output)) = self.previewing else {
            return Ok(());
        };
        let length = self.preview_length;
        let live = self
            .outputs
            .get_mut(output)
            .and_then(|o| o.live.as_mut())
            .ok_or(SessionError::NoOutput)?;
        let seconds = if seconds.is_finite() { seconds } else { 0.0 };
        let position =
            sbj_engine::seconds_to_samples(seconds.clamp(0.0, length), live.config.sample_rate);
        live.controller.seek(PREVIEW_PLAYER, position)?;
        live.controller.play(PREVIEW_PLAYER)?;
        Ok(())
    }

    pub fn stop_preview(&mut self) {
        self.previewing_file = None;
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
            file: self.previewing_file.clone(),
            playing: live.controller.is_playing(PREVIEW_PLAYER),
            position_seconds: samples_to_seconds(
                live.controller.position(PREVIEW_PLAYER),
                live.config.sample_rate,
            ),
            duration_seconds: self.preview_length,
        })
    }
}
