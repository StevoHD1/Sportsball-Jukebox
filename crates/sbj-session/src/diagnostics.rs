//! The event log: glitches (with what was playing), device losses and
//! reconnections, failovers, failed fires and other warnings, with times,
//! for working out what went wrong. Kept in memory (the most recent 500)
//! and, with a log folder set, appended to `events-YYYY-MM-DD.jsonl`
//! there (UTC days), next to the play log.

use std::collections::VecDeque;
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use sbj_backend::{AudioBackend, OutputStream};
use serde::Serialize;

use crate::Session;
use crate::log::utc_date;

/// Events kept in memory.
const EVENT_LIMIT: usize = 500;

/// One logged event.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EventEntry {
    /// Unix time, seconds.
    pub time: f64,
    /// "info", "warning" or "error".
    pub level: &'static str,
    /// "glitch", "device", "fire", "show" or "notice".
    pub kind: &'static str,
    pub message: String,
}

impl<B: AudioBackend> Session<B> {
    /// Recent events, oldest first.
    pub fn events(&self) -> &VecDeque<EventEntry> {
        &self.events
    }

    pub(crate) fn log_event(&mut self, level: &'static str, kind: &'static str, message: String) {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64());
        let entry = EventEntry {
            time,
            level,
            kind,
            message,
        };
        if let Some(dir) = &self.log.dir {
            let line = serde_json::to_string(&entry).unwrap_or_default();
            let path = dir.join(format!("events-{}.jsonl", utc_date(time)));
            let written = std::fs::create_dir_all(dir).and_then(|()| {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .and_then(|mut f| writeln!(f, "{line}"))
            });
            if let Err(e) = written {
                self.log.last_error = Some(format!("could not write the event log: {e}"));
            }
        }
        self.events.push_back(entry);
        while self.events.len() > EVENT_LIMIT {
            self.events.pop_front();
        }
    }

    /// A message for the operator (shown once) that is also logged.
    pub(crate) fn notify(&mut self, message: String) {
        self.log_event("warning", "notice", message.clone());
        self.notices.push(message);
    }

    /// Logs new glitches on each output, with what was playing (called
    /// from `poll`).
    pub(crate) fn check_glitches(&mut self) {
        for index in 0..self.outputs.len() {
            let Some(raw) = self.outputs[index]
                .live
                .as_ref()
                .map(|l| l.stream.status().xruns())
            else {
                continue;
            };
            let seen = self.outputs[index].seen_xruns;
            if raw <= seen {
                continue;
            }
            self.outputs[index].seen_xruns = raw;
            let new = raw - seen;
            let device = self.outputs[index]
                .live
                .as_ref()
                .map(|l| l.device.name.clone())
                .unwrap_or_default();
            let name = self
                .show
                .outputs
                .get(index)
                .map(|o| o.name.clone())
                .unwrap_or_default();
            let playing: Vec<String> = self
                .show
                .cues
                .iter()
                .filter(|c| self.loaded.get(&c.slot).is_some_and(|l| l.output == index))
                .filter(|c| self.cue_state(c.slot).is_ok_and(|s| s.playing))
                .map(|c| format!("\"{}\"", c.title))
                .collect();
            let what = if playing.is_empty() {
                "nothing was playing".to_string()
            } else {
                format!("playing: {}", playing.join(", "))
            };
            let count = if new == 1 {
                "A glitch".to_string()
            } else {
                format!("{new} glitches")
            };
            self.log_event(
                "warning",
                "glitch",
                format!("{count} on output \"{name}\" ({device}); {what}."),
            );
        }
    }

    /// Starts each output's glitch count again from zero (the log keeps
    /// the history).
    pub fn reset_glitches(&mut self) {
        for output in &mut self.outputs {
            if let Some(live) = &output.live {
                output.glitch_offset = live.stream.status().xruns();
            }
        }
        self.log_event("info", "glitch", "Glitch counts reset.".into());
    }
}
