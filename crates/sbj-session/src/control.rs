//! External control: MIDI and OSC messages mapped to session actions.
//!
//! Listeners (in `sbj-control`) turn raw input into [`ControlMessage`]s
//! and call [`Session::handle_control`] directly from their own threads,
//! so a trigger fires within microseconds rather than waiting for the UI's
//! next poll.
//!
//! OSC address space (pads and banks are 1-based; the default bank is the
//! one currently shown):
//!
//! | Address | Action |
//! |---|---|
//! | `/sbj/fire/<pad>` or `/sbj/fire/<bank>/<pad>` | fire a pad |
//! | `/sbj/fire` with an int argument | fire that pad in the current bank |
//! | `/sbj/go/<pad>` | same as fire; on a vamping pad, lets it continue |
//! | `/sbj/title` with a string argument | fire the pad with that title |
//! | `/sbj/stop/<pad>` or `/sbj/stop/<bank>/<pad>` | stop a pad |
//! | `/sbj/fade/<pad>` or `/sbj/fade/<bank>/<pad>` | fade a pad out |
//! | `/sbj/next/<pad>` or `/sbj/next/<bank>/<pad>` | playlist pad: next song |
//! | `/sbj/stopall` | fade everything out |
//! | `/sbj/stopall/now` | stop everything immediately |
//! | `/sbj/bank/<n>`, `/sbj/bank/next`, `/sbj/bank/prev` | change bank |
//!
//! A message whose first argument is the number 0 is ignored: buttons on
//! OSC controllers (TouchOSC, Stream Deck plugins) send 1 on press and 0
//! on release, and only the press should act.

use sbj_backend::AudioBackend;

use crate::show::{MidiKind, MidiTrigger};
use crate::{Result, Session, SessionError};

/// A MIDI message we can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MidiMessage {
    pub kind: MidiKind,
    pub number: u8,
    pub value: u8,
    /// 1-16.
    pub channel: u8,
}

impl MidiMessage {
    /// Parses raw MIDI bytes. Only "press" messages are returned: note-on
    /// with velocity > 0, control change with value > 0, and program
    /// change. Note-offs, releases and everything else give `None`.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let status = *bytes.first()?;
        let channel = (status & 0x0F) + 1;
        let data = |i: usize| bytes.get(i).copied().filter(|b| *b < 0x80);
        let (kind, number, value) = match status & 0xF0 {
            0x90 => (MidiKind::Note, data(1)?, data(2)?),
            0xB0 => (MidiKind::Cc, data(1)?, data(2)?),
            0xC0 => (MidiKind::Program, data(1)?, 127),
            _ => return None,
        };
        (value > 0).then_some(Self {
            kind,
            number,
            value,
            channel,
        })
    }

    /// A trigger matching this message on this channel.
    pub fn trigger(&self) -> MidiTrigger {
        MidiTrigger {
            kind: self.kind,
            number: self.number,
            channel: Some(self.channel),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum OscArg {
    Int(i64),
    Float(f64),
    Str(String),
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ControlMessage {
    Midi(MidiMessage),
    Osc { address: String, args: Vec<OscArg> },
}

/// What MIDI learn will assign the next MIDI message to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LearnTarget {
    Cue(u32),
    StopAll,
    NextBank,
    PrevBank,
}

impl<B: AudioBackend> Session<B> {
    /// Assigns the next incoming MIDI message to `target`.
    pub fn start_learning(&mut self, target: LearnTarget) -> Result<()> {
        if let LearnTarget::Cue(slot) = target
            && self.show.cue(slot).is_none()
        {
            return Err(SessionError::InvalidCue(slot));
        }
        self.learning = Some(target);
        Ok(())
    }

    pub fn cancel_learning(&mut self) {
        self.learning = None;
    }

    pub fn learning(&self) -> Option<LearnTarget> {
        self.learning
    }

    /// The most recent control message and what it did, for display.
    pub fn last_control(&self) -> Option<&str> {
        self.last_control.as_deref()
    }

    /// Sets (or clears) a show-wide MIDI control.
    pub fn set_control_trigger(&mut self, target: LearnTarget, trigger: Option<MidiTrigger>) {
        let controls = &mut self.show.controls;
        match target {
            LearnTarget::StopAll => controls.stop_all = trigger,
            LearnTarget::NextBank => controls.next_bank = trigger,
            LearnTarget::PrevBank => controls.prev_bank = trigger,
            LearnTarget::Cue(slot) => {
                if let Some(cue) = self.show.cues.iter_mut().find(|c| c.slot == slot) {
                    cue.midi = trigger;
                }
            }
        }
        self.dirty = true;
    }

    /// Acts on a MIDI or OSC message. Errors (e.g. firing a pad whose
    /// output is down) are reported through [`Session::last_control`].
    pub fn handle_control(&mut self, message: ControlMessage) {
        self.log.source = match &message {
            ControlMessage::Midi(_) => "midi",
            ControlMessage::Osc { .. } => "osc",
        };
        let outcome = match &message {
            ControlMessage::Midi(midi) => self.handle_midi(*midi),
            ControlMessage::Osc { address, args } => self.handle_osc(address, args),
        };
        self.log.source = "operator";
        let source = match &message {
            ControlMessage::Midi(m) => format!("MIDI {}", m.trigger().describe()),
            ControlMessage::Osc { address, .. } => format!("OSC {address}"),
        };
        if let Some(outcome) = outcome {
            self.last_control = Some(format!("{source} → {outcome}"));
        }
    }

    fn handle_midi(&mut self, midi: MidiMessage) -> Option<String> {
        if let Some(target) = self.learning.take() {
            let trigger = midi.trigger();
            self.set_control_trigger(target, Some(trigger));
            return Some(format!("learned for {}", self.describe_target(target)));
        }
        let matches = |t: &Option<MidiTrigger>| {
            t.is_some_and(|t| t.matches(midi.kind, midi.number, midi.channel))
        };
        let controls = self.show.controls.clone();
        if matches(&controls.stop_all) {
            return Some(report(
                self.stop_all(self.show.stop_all_fade),
                "stop all".into(),
            ));
        }
        if matches(&controls.next_bank) {
            self.step_bank(true);
            return Some(format!("bank {}", self.current_bank + 1));
        }
        if matches(&controls.prev_bank) {
            self.step_bank(false);
            return Some(format!("bank {}", self.current_bank + 1));
        }
        let slots: Vec<u32> = self
            .show
            .cues
            .iter()
            .filter(|c| matches(&c.midi))
            .map(|c| c.slot)
            .collect();
        if slots.is_empty() {
            return Some("not assigned".into());
        }
        let mut done = Vec::new();
        for slot in slots {
            let label = self.describe_target(LearnTarget::Cue(slot));
            done.push(report(self.fire_cue(slot), format!("fire {label}")));
        }
        Some(done.join(", "))
    }

    fn handle_osc(&mut self, address: &str, args: &[OscArg]) -> Option<String> {
        // Button release.
        if matches!(args.first(), Some(OscArg::Int(0)))
            || matches!(args.first(), Some(OscArg::Float(f)) if *f == 0.0)
        {
            return None;
        }
        let parts: Vec<String> = address
            .trim_matches('/')
            .split('/')
            .map(str::to_ascii_lowercase)
            .collect();
        let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
        let first_int = args.iter().find_map(|a| match a {
            OscArg::Int(i) => Some(*i),
            OscArg::Float(f) if f.fract() == 0.0 => Some(*f as i64),
            _ => None,
        });

        let outcome = match parts.as_slice() {
            ["sbj", "fire"] => match first_int.and_then(|pad| self.osc_slot(None, pad)) {
                Some(slot) => self.osc_fire(slot),
                None => "needs a pad number".into(),
            },
            [
                "sbj",
                verb @ ("fire" | "go" | "stop" | "fade" | "next"),
                rest @ ..,
            ] => {
                let numbers: Vec<i64> = rest.iter().filter_map(|p| p.parse().ok()).collect();
                let slot = match numbers.as_slice() {
                    [pad] => self.osc_slot(None, *pad),
                    [bank, pad] => self.osc_slot(Some(*bank), *pad),
                    _ => None,
                };
                match (slot, *verb) {
                    (None, _) => "no such pad".into(),
                    // "go" is "fire" by another name: on a vamping pad it
                    // releases the vamp.
                    (Some(slot), "fire" | "go") => self.osc_fire(slot),
                    (Some(slot), "next") => {
                        let label = self.describe_target(LearnTarget::Cue(slot));
                        report(self.skip_cue(slot), format!("next song on {label}"))
                    }
                    (Some(slot), "stop") => {
                        let label = self.describe_target(LearnTarget::Cue(slot));
                        report(self.stop_cue(slot, 0.0), format!("stop {label}"))
                    }
                    (Some(slot), _) => {
                        let label = self.describe_target(LearnTarget::Cue(slot));
                        let fade = self
                            .show
                            .cue(slot)
                            .map_or(crate::DEFAULT_FADE_OUT, crate::CueDoc::fade_out_seconds);
                        report(self.stop_cue(slot, fade), format!("fade {label}"))
                    }
                }
            }
            ["sbj", "title"] => {
                let title = args.iter().find_map(|a| match a {
                    OscArg::Str(s) => Some(s.to_lowercase()),
                    _ => None,
                });
                let slot = title.and_then(|t| {
                    self.show
                        .cues
                        .iter()
                        .find(|c| c.title.to_lowercase() == t)
                        .map(|c| c.slot)
                });
                match slot {
                    Some(slot) => self.osc_fire(slot),
                    None => "no pad with that title".into(),
                }
            }
            ["sbj", "stopall"] => report(self.stop_all(self.show.stop_all_fade), "stop all".into()),
            ["sbj", "stopall", "now"] => report(self.stop_all(0.0), "stop all now".into()),
            ["sbj", "bank", "next"] => {
                self.step_bank(true);
                format!("bank {}", self.current_bank + 1)
            }
            ["sbj", "bank", "prev"] => {
                self.step_bank(false);
                format!("bank {}", self.current_bank + 1)
            }
            ["sbj", "bank", n] => match n.parse::<u32>() {
                Ok(n) if n >= 1 => report(self.set_bank(n - 1), format!("bank {n}")),
                _ => "no such bank".into(),
            },
            _ => "unknown address".into(),
        };
        Some(outcome)
    }

    /// 1-based (bank, pad) to a slot, defaulting to the current bank.
    fn osc_slot(&self, bank: Option<i64>, pad: i64) -> Option<u32> {
        let bank = match bank {
            Some(b) if b >= 1 && (b as usize) <= self.show.banks.len() => (b - 1) as u32,
            Some(_) => return None,
            None => self.current_bank,
        };
        (pad >= 1 && pad <= i64::from(self.show.slots))
            .then(|| bank * self.show.slots + (pad - 1) as u32)
    }

    fn osc_fire(&mut self, slot: u32) -> String {
        let label = self.describe_target(LearnTarget::Cue(slot));
        if self.show.cue(slot).is_none() {
            return format!("{label} is empty");
        }
        report(self.fire_cue(slot), format!("fire {label}"))
    }

    fn describe_target(&self, target: LearnTarget) -> String {
        match target {
            LearnTarget::StopAll => "Stop All".into(),
            LearnTarget::NextBank => "Next Bank".into(),
            LearnTarget::PrevBank => "Previous Bank".into(),
            LearnTarget::Cue(slot) => match self.show.cue(slot) {
                Some(c) if !c.title.is_empty() => format!("\"{}\"", c.title),
                _ => {
                    let slots = self.show.slots.max(1);
                    format!("bank {} pad {}", slot / slots + 1, slot % slots + 1)
                }
            },
        }
    }
}

/// Describes the outcome of an action for the control log.
fn report(result: Result<()>, done: String) -> String {
    match result {
        Ok(()) => done,
        Err(e) => format!("{done} failed: {e}"),
    }
}
