//! The pre-game check: everything an operator should know before doors
//! open, in one list.

use std::collections::HashMap;

use sbj_backend::AudioBackend;

use crate::show::MidiTrigger;
use crate::{OutputStatus, Session};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CheckLevel {
    Ok,
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub level: CheckLevel,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preflight {
    pub checks: Vec<Check>,
    /// Decoded audio held in memory.
    pub memory_bytes: u64,
}

impl Preflight {
    /// No errors (warnings are allowed).
    pub fn ready(&self) -> bool {
        self.checks.iter().all(|c| c.level < CheckLevel::Error)
    }
}

/// More audio than this in memory earns a warning.
const MEMORY_WARNING_BYTES: u64 = 4 << 30;

impl<B: AudioBackend> Session<B> {
    pub fn preflight(&self) -> Preflight {
        let mut checks = Vec::new();
        let mut add = |level, message: String| checks.push(Check { level, message });

        // Outputs.
        for output in self.outputs_state() {
            let device = output.device_name.clone().unwrap_or_default();
            match output.status {
                OutputStatus::Running => add(
                    CheckLevel::Ok,
                    format!(
                        "Output \"{}\" is playing through {device} ({:.1} kHz).",
                        output.name,
                        f64::from(output.sample_rate) / 1000.0
                    ),
                ),
                OutputStatus::OnBackup => add(
                    CheckLevel::Warning,
                    format!(
                        "Output \"{}\" is on its backup device ({device}); its main device is missing.",
                        output.name
                    ),
                ),
                OutputStatus::Offline => add(
                    CheckLevel::Error,
                    format!(
                        "Output \"{}\" has no audio device: {}",
                        output.name,
                        output.error.as_deref().unwrap_or("not connected")
                    ),
                ),
            }
            if output.xruns > 0 {
                add(
                    CheckLevel::Warning,
                    format!(
                        "Output \"{}\" has glitched {} time(s) since it was opened.",
                        output.name, output.xruns
                    ),
                );
            }
        }

        // Cues.
        let slots = self.show.slots.max(1);
        let pad_name = |slot: u32| {
            let title = self
                .show
                .cue(slot)
                .map(|c| c.title.clone())
                .unwrap_or_default();
            let where_ = if self.show.banks.len() > 1 {
                format!("bank {} pad {}", slot / slots + 1, slot % slots + 1)
            } else {
                format!("pad {}", slot + 1)
            };
            if title.is_empty() {
                where_
            } else {
                format!("\"{title}\" ({where_})")
            }
        };
        if self.show.cues.is_empty() {
            add(CheckLevel::Warning, "The show has no pads assigned.".into());
        }
        for problem in &self.problems {
            add(
                CheckLevel::Error,
                format!("{}: {}", pad_name(problem.slot), problem.message),
            );
        }
        let mut not_ready = 0;
        for cue in &self.show.cues {
            if cue.clips.is_empty() {
                add(
                    CheckLevel::Warning,
                    format!("{} has no audio.", pad_name(cue.slot)),
                );
            }
            if !self.loaded.contains_key(&cue.slot) {
                not_ready += 1;
            }
        }
        if not_ready > 0 {
            add(
                CheckLevel::Error,
                format!("{not_ready} pad(s) cannot play because their output is down."),
            );
        }

        // Duplicate hotkeys (keys only act in the visible bank).
        let mut keys: HashMap<(u32, String), Vec<u32>> = HashMap::new();
        for cue in &self.show.cues {
            if let Some(key) = cue.hotkey.as_ref().filter(|k| !k.is_empty()) {
                keys.entry((cue.slot / slots, key.to_lowercase()))
                    .or_default()
                    .push(cue.slot);
            }
        }
        let mut dup_keys: Vec<_> = keys.into_iter().filter(|(_, v)| v.len() > 1).collect();
        dup_keys.sort();
        for ((_, key), pads) in dup_keys {
            let names: Vec<String> = pads.into_iter().map(pad_name).collect();
            add(
                CheckLevel::Warning,
                format!(
                    "Key \"{}\" is used by {}.",
                    key.to_uppercase(),
                    names.join(" and ")
                ),
            );
        }

        // Duplicate MIDI triggers.
        let mut midi: HashMap<MidiTrigger, Vec<String>> = HashMap::new();
        for cue in &self.show.cues {
            if let Some(t) = cue.midi {
                midi.entry(t).or_default().push(pad_name(cue.slot));
            }
        }
        let controls = &self.show.controls;
        for (name, trigger) in [
            ("Stop All", controls.stop_all),
            ("Next Bank", controls.next_bank),
            ("Previous Bank", controls.prev_bank),
        ] {
            if let Some(t) = trigger {
                midi.entry(t).or_default().push(name.into());
            }
        }
        let mut dup_midi: Vec<_> = midi.into_iter().filter(|(_, v)| v.len() > 1).collect();
        dup_midi.sort_by_key(|(t, _)| t.describe());
        for (trigger, names) in dup_midi {
            add(
                CheckLevel::Warning,
                format!(
                    "MIDI {} triggers {}.",
                    trigger.describe(),
                    names.join(" and ")
                ),
            );
        }

        // Memory.
        let memory_bytes: u64 = self.library.values().map(|s| s.memory_bytes()).sum();
        let mb = memory_bytes as f64 / (1024.0 * 1024.0);
        add(
            if memory_bytes > MEMORY_WARNING_BYTES {
                CheckLevel::Warning
            } else {
                CheckLevel::Info
            },
            format!(
                "{} audio file(s) loaded, using {mb:.0} MB of memory.",
                self.library.len()
            ),
        );

        checks.sort_by_key(|c| std::cmp::Reverse(c.level));
        Preflight {
            checks,
            memory_bytes,
        }
    }
}
