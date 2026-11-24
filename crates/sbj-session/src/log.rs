//! The play log: what played, when, for how long, and what triggered it.
//! Used for music-licensing and sponsor reports.
//!
//! Entries open when a pad (or playlist song) starts and close when it
//! stops, finishes, or is replaced. Closed entries are appended as JSON
//! lines to a file per UTC day in the log directory (if one is set), so
//! nothing is lost if the app quits mid-game. All file I/O happens on the
//! control thread, in `poll`.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use sbj_backend::AudioBackend;
use serde::{Deserialize, Serialize};

use crate::Session;

/// How many entries are kept in memory for display.
const MEMORY_LIMIT: usize = 5_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogEntry {
    /// Unix time the sound started, seconds.
    pub started: f64,
    /// Seconds it played (0 while still playing).
    pub seconds: f64,
    #[serde(default)]
    pub game: String,
    #[serde(default)]
    pub show: String,
    pub slot: u32,
    /// 1-based, for people.
    pub bank: u32,
    pub pad: u32,
    pub title: String,
    /// The audio file that played (a playlist's song, or the pad's first
    /// clip).
    pub file: String,
    #[serde(default)]
    pub sponsor: Option<String>,
    /// "operator", "midi", "osc" or "auto" (playlist auto-advance).
    pub source: String,
    pub output: String,
    /// Still playing.
    #[serde(default)]
    pub open: bool,
}

/// Totals for one song or sponsor.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LogTotal {
    pub name: String,
    pub plays: u32,
    pub seconds: f64,
}

#[derive(Debug, Default)]
pub(crate) struct PlayLog {
    entries: Vec<LogEntry>,
    /// Index into `entries` of each slot's open entry (and its item).
    open: HashMap<u32, (usize, Option<usize>)>,
    pub(crate) dir: Option<PathBuf>,
    pub(crate) game: String,
    /// What is triggering actions right now.
    pub(crate) source: &'static str,
    pub(crate) last_error: Option<String>,
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

/// `YYYY-MM-DD` (UTC) for a unix time.
pub(crate) fn utc_date(unix: f64) -> String {
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let days = (unix / 86_400.0).floor() as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

impl PlayLog {
    pub(crate) fn new() -> Self {
        Self {
            source: "operator",
            ..Self::default()
        }
    }

    fn close(&mut self, slot: u32, at: f64) {
        let Some((index, _)) = self.open.remove(&slot) else {
            return;
        };
        let Some(entry) = self.entries.get_mut(index) else {
            return;
        };
        entry.open = false;
        entry.seconds = (at - entry.started).max(0.0);
        let line = serde_json::to_string(entry).unwrap_or_default();
        if let Some(dir) = &self.dir {
            let path = dir.join(format!("{}.jsonl", utc_date(entry.started)));
            let written = std::fs::create_dir_all(dir).and_then(|()| {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .and_then(|mut f| writeln!(f, "{line}"))
            });
            if let Err(e) = written {
                self.last_error = Some(format!("could not write the play log: {e}"));
            }
        }
    }

    fn trim(&mut self) {
        if self.entries.len() > MEMORY_LIMIT {
            let drop = self.entries.len() - MEMORY_LIMIT;
            self.entries.drain(..drop);
            for (index, _) in self.open.values_mut() {
                *index = index.saturating_sub(drop);
            }
        }
    }
}

impl<B: AudioBackend> Session<B> {
    /// Directory for daily log files (`YYYY-MM-DD.jsonl`, UTC days).
    pub fn set_log_dir(&mut self, dir: Option<PathBuf>) {
        self.log.dir = dir;
    }

    /// Name of the current game, stamped on new log entries.
    pub fn set_game(&mut self, name: &str) {
        self.log.game = name.trim().to_string();
    }

    pub fn game(&self) -> &str {
        &self.log.game
    }

    /// Log entries, oldest first (open ones show seconds so far).
    pub fn log_entries(&self) -> Vec<LogEntry> {
        let at = now();
        self.log
            .entries
            .iter()
            .map(|e| {
                let mut e = e.clone();
                if e.open {
                    e.seconds = (at - e.started).max(0.0);
                }
                e
            })
            .collect()
    }

    /// Forgets the in-memory log (files on disk are kept).
    pub fn clear_log(&mut self) {
        let at = now();
        let slots: Vec<u32> = self.log.open.keys().copied().collect();
        for slot in slots {
            self.log.close(slot, at);
        }
        self.log.entries.clear();
    }

    /// Totals per song (title + file) and per sponsor.
    pub fn log_totals(&self) -> (Vec<LogTotal>, Vec<LogTotal>) {
        let mut songs: HashMap<String, LogTotal> = HashMap::new();
        let mut sponsors: HashMap<String, LogTotal> = HashMap::new();
        for e in self.log_entries() {
            let file_name = std::path::Path::new(&e.file)
                .file_stem()
                .map_or_else(|| e.file.clone(), |s| s.to_string_lossy().into_owned());
            let song = if file_name.is_empty() || file_name == e.title {
                e.title.clone()
            } else {
                format!("{} ({file_name})", e.title)
            };
            let total = songs.entry(song.clone()).or_insert(LogTotal {
                name: song,
                plays: 0,
                seconds: 0.0,
            });
            total.plays += 1;
            total.seconds += e.seconds;
            if let Some(sponsor) = e.sponsor.filter(|s| !s.is_empty()) {
                let total = sponsors.entry(sponsor.clone()).or_insert(LogTotal {
                    name: sponsor,
                    plays: 0,
                    seconds: 0.0,
                });
                total.plays += 1;
                total.seconds += e.seconds;
            }
        }
        let sorted = |m: HashMap<String, LogTotal>| {
            let mut v: Vec<LogTotal> = m.into_values().collect();
            v.sort_by(|a, b| b.plays.cmp(&a.plays).then(a.name.cmp(&b.name)));
            v
        };
        (sorted(songs), sorted(sponsors))
    }

    /// A problem writing the log file, once.
    pub fn take_log_error(&mut self) -> Option<String> {
        self.log.last_error.take()
    }

    /// Records that `slot` (playlist `item`, if any) just started.
    pub(crate) fn log_start(&mut self, slot: u32, item: Option<usize>) {
        let at = now();
        self.log.close(slot, at);
        let Some(cue) = self.show.cue(slot) else {
            return;
        };
        let file = match (&cue.playlist, item) {
            (Some(p), Some(i)) => p.items.get(i).cloned().unwrap_or_default(),
            _ => cue
                .clips
                .first()
                .map(|c| c.file.clone())
                .unwrap_or_default(),
        };
        let per_bank = self.show.slots.max(1);
        let output = self
            .show
            .outputs
            .get(cue.output as usize)
            .map_or_else(String::new, |o| o.name.clone());
        let entry = LogEntry {
            started: at,
            seconds: 0.0,
            game: self.log.game.clone(),
            show: self.show.name.clone(),
            slot,
            bank: slot / per_bank + 1,
            pad: slot % per_bank + 1,
            title: cue.title.clone(),
            file,
            sponsor: cue.sponsor.clone(),
            source: self.log.source.to_string(),
            output,
            open: true,
        };
        self.log.entries.push(entry);
        let index = self.log.entries.len() - 1;
        self.log.open.insert(slot, (index, item));
        self.log.trim();
    }

    /// Closes entries whose sound has stopped or moved on. Called on poll.
    pub(crate) fn update_log(&mut self) {
        let at = now();
        let finished: Vec<u32> = self
            .log
            .open
            .iter()
            .filter(|(slot, (_, item))| {
                let state = self.cue_state(**slot).unwrap_or_default();
                !state.playing || (item.is_some() && state.item.map(|i| i as usize) != *item)
            })
            .map(|(slot, _)| *slot)
            .collect();
        for slot in finished {
            self.log.close(slot, at);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::utc_date;

    #[test]
    fn utc_dates_match_the_calendar() {
        for (t, d) in [
            (0.0, "1970-01-01"),
            (951_782_400.0, "2000-02-29"),
            (1_764_000_000.0, "2025-11-24"),
            (1_798_761_599.0, "2026-12-31"),
            (4_102_444_800.0, "2100-01-01"),
        ] {
            assert_eq!(utc_date(t), d, "{t}");
        }
    }
}
