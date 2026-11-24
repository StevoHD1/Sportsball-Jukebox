//! The phone/tablet remote's view of the session, and its requests.
//!
//! The HTTP side lives in `sbj-control`; this module only turns a
//! `(method, path)` into a JSON reply. Button presses go through
//! [`Session::handle_control`] as [`ControlMessage::Remote`], so they are
//! logged with the source "remote" and show in the control footer just
//! like MIDI and OSC.

use serde_json::{Value, json};
use std::path::Path;

use crate::Session;
use crate::control::ControlMessage;
use sbj_backend::AudioBackend;

/// Why a remote request was refused: an HTTP status and a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteError {
    pub status: u16,
    pub message: String,
}

impl RemoteError {
    fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

fn file_stem(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map_or_else(|| path.to_string(), |s| s.to_string_lossy().into_owned())
}

impl<B: AudioBackend> Session<B> {
    /// What the remote page shows: the current bank's pads, and every
    /// playing pad in any bank.
    pub fn remote_state(&self) -> Value {
        let slots = self.show.slots.max(1);
        let total = self.show.total_slots();
        let first = self.current_bank * slots;

        let pad = |slot: u32| -> Option<Value> {
            let state = self.cue_state(slot).ok()?;
            let Some(cue) = self.show.cue(slot) else {
                return Some(json!({"slot": slot, "assigned": false}));
            };
            let playlist = cue.playlist.as_ref().filter(|p| !p.items.is_empty());
            let line = match playlist {
                Some(p) => match state.item.and_then(|i| p.items.get(i as usize)) {
                    Some(song) => format!(
                        "♫ {}/{} · {}",
                        state.item.unwrap_or(0) + 1,
                        p.items.len(),
                        file_stem(song)
                    ),
                    None => format!("♫ {} songs", p.items.len()),
                },
                None => cue.hotkey.clone().unwrap_or_default().to_uppercase(),
            };
            let remaining = (!state.looping && state.duration_seconds > 0.0)
                .then(|| (state.duration_seconds - state.position_seconds).max(0.0));
            let progress = if state.duration_seconds > 0.0 {
                (state.position_seconds / state.duration_seconds).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let title = if cue.title.is_empty() {
                format!("Pad {}", slot % slots + 1)
            } else {
                cue.title.clone()
            };
            Some(json!({
                "slot": slot,
                "assigned": true,
                "title": title,
                "line": line,
                "color": cue.color.as_deref().unwrap_or("blue"),
                "ready": state.ready,
                "playing": state.playing,
                "holding": state.holding,
                "progress": progress,
                "remaining": remaining,
            }))
        };

        let pads: Vec<Value> = (first..(first + slots).min(total))
            .filter_map(pad)
            .collect();
        let mut playing: Vec<Value> = self
            .show
            .cues
            .iter()
            .map(|c| c.slot)
            .filter(|&slot| self.cue_state(slot).is_ok_and(|s| s.playing))
            .filter_map(pad)
            .collect();
        // Soonest to end first; loops last.
        playing.sort_by(|a, b| {
            let key = |v: &Value| v["remaining"].as_f64().unwrap_or(f64::INFINITY);
            key(a).total_cmp(&key(b))
        });

        json!({
            "show": self.show.name,
            "banks": self.show.banks,
            "bank": self.current_bank,
            "columns": self.show.columns.max(1),
            "pads": pads,
            "playing": playing,
            "last_control": self.last_control,
        })
    }

    /// Answers a remote request, e.g. `("GET", "/api/state")` or
    /// `("POST", "/api/fire/3")` (slots are 0-based and absolute). Every
    /// successful request returns the new [`Session::remote_state`].
    pub fn handle_remote(&mut self, method: &str, path: &str) -> Result<Value, RemoteError> {
        let parts: Vec<&str> = path
            .trim_start_matches("/api")
            .trim_matches('/')
            .split('/')
            .collect();
        let address = match (method, parts.as_slice()) {
            ("GET", ["state"]) => return Ok(self.remote_state()),
            ("POST", [verb @ ("fire" | "stop" | "fade" | "next"), slot]) => {
                let slot: u32 = slot
                    .parse()
                    .map_err(|_| RemoteError::new(400, "bad pad number"))?;
                if slot >= self.show.total_slots() {
                    return Err(RemoteError::new(404, "no such pad"));
                }
                let slots = self.show.slots.max(1);
                format!("/sbj/{verb}/{}/{}", slot / slots + 1, slot % slots + 1)
            }
            ("POST", ["stopall"]) => "/sbj/stopall".into(),
            ("POST", ["stopall", "now"]) => "/sbj/stopall/now".into(),
            ("POST", ["bank", n]) => format!("/sbj/bank/{n}"),
            (_, _) => return Err(RemoteError::new(404, "not found")),
        };
        self.handle_control(ControlMessage::Remote { address });
        Ok(self.remote_state())
    }
}
