//! The show document: everything the operator sets up before a game,
//! saved as versioned JSON.
//!
//! Times are in **seconds** so a show does not depend on any device's
//! sample rate; they are converted to sample frames when a cue is loaded
//! onto an output. The Swift app mirrors these types with `Codable`
//! (`apple/SportsballJukebox/Show.swift`) using snake_case keys, so field
//! names here are part of the file format and the app contract.
//!
//! Compatibility rules: new fields must be optional with defaults, so old
//! files keep loading. Breaking changes bump [`SHOW_VERSION`] and add a
//! migration in [`Show::from_json`].

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sbj_engine::{
    AudioBuffer, Clip, Fade, FadeCurve, LoopCount, Marker, MarkerAction, MarkerId, Samples,
    Timeline, seconds_to_samples,
};
use serde::{Deserialize, Serialize};

pub const SHOW_FORMAT: &str = "sportsball-jukebox-show";
pub const SHOW_VERSION: u32 = 1;
/// Upper bound on pads across all banks (and engine players per output).
pub const MAX_SLOTS: u32 = 512;
/// Upper bound on pads in one bank.
pub const MAX_PADS_PER_BANK: u32 = 128;
/// Prefix of the problem reported for a clip whose audio is not loaded.
pub const MISSING_FILE: &str = "missing audio file: ";
/// Marker id reserved for a cue's whole-cue loop.
const WHOLE_CUE_LOOP_ID: u64 = u64::MAX;

fn one() -> f32 {
    1.0
}
fn default_slots() -> u32 {
    24
}
fn default_columns() -> u32 {
    6
}
fn default_banks() -> Vec<String> {
    vec!["Bank 1".into()]
}
fn is_default_banks(banks: &[String]) -> bool {
    banks.len() == 1 && banks[0] == "Bank 1"
}
fn is_default_controls(c: &ControlsDoc) -> bool {
    *c == ControlsDoc::default()
}
fn default_outputs() -> Vec<OutputDoc> {
    vec![OutputDoc::named("Main")]
}
fn is_false(b: &bool) -> bool {
    !*b
}
fn is_zero(x: &f64) -> bool {
    *x == 0.0
}
fn is_zero_u32(x: &u32) -> bool {
    *x == 0
}
fn is_one(x: &f32) -> bool {
    *x == 1.0
}

/// A whole show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Show {
    pub format: String,
    pub version: u32,
    #[serde(default)]
    pub name: String,
    /// Number of pads in each bank's grid.
    #[serde(default = "default_slots")]
    pub slots: u32,
    /// Pages of pads. A cue's `slot` is `bank * slots + pad`.
    #[serde(default = "default_banks", skip_serializing_if = "is_default_banks")]
    pub banks: Vec<String>,
    #[serde(default = "default_columns")]
    pub columns: u32,
    /// Logical outputs ("Main", "Locker room", ...). Cues route to one by
    /// index. Always at least one.
    #[serde(default = "default_outputs")]
    pub outputs: Vec<OutputDoc>,
    #[serde(default)]
    pub cues: Vec<CueDoc>,
    /// MIDI triggers for show-wide actions.
    #[serde(default, skip_serializing_if = "is_default_controls")]
    pub controls: ControlsDoc,
    /// Fade used when an exclusive group hands over to a new cue, seconds.
    #[serde(
        default = "default_group_fade",
        skip_serializing_if = "is_default_group_fade"
    )]
    pub group_fade: f64,
    /// Fade used by Stop All (button, Esc, MIDI, OSC), seconds.
    #[serde(
        default = "default_stop_all_fade",
        skip_serializing_if = "is_default_stop_all_fade"
    )]
    pub stop_all_fade: f64,
    /// Output used to preview (audition) pads, e.g. headphones; `None` =
    /// preview off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_output: Option<u32>,
    /// Play every pad at this loudness (LUFS, e.g. -16) unless the pad
    /// opts out; `None` = off (each file plays as it is).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level_match: Option<f64>,
    /// The game's running order, fired step by step with GO.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rundown: Vec<RundownItem>,
    /// Audio files kept with the show for building pads, besides those
    /// pads already use (see [`Show::library_files`]). Not decoded until a
    /// pad uses them or they are previewed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub library: Vec<String>,
    /// Settings given to new pads when audio is dropped on an empty pad.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pad_defaults: Option<PadDefaults>,
    /// Banks (0-based) with Multiplay off: firing a pad fades out the
    /// bank's other pads.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub single_play: Vec<u32>,
}

/// Settings for new pads (applied by the app when it creates a pad).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PadDefaults {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub fade_in: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_out: Option<f64>,
    #[serde(default, rename = "loop", skip_serializing_if = "is_false")]
    pub looping: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub output: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duck: Option<f32>,
    /// Start new pads where the sound starts.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub trim_silence: bool,
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub level_match: bool,
}

impl Default for PadDefaults {
    fn default() -> Self {
        Self {
            color: None,
            fade_in: 0.0,
            fade_out: None,
            looping: false,
            group: None,
            output: 0,
            duck: None,
            trim_silence: true,
            level_match: true,
        }
    }
}

/// Most a quiet file is boosted by level matching, dB.
pub const LEVEL_MATCH_MAX_BOOST_DB: f64 = 12.0;
/// Level matching never pushes a file's peak above this, dBFS.
pub const LEVEL_MATCH_CEILING_DBFS: f64 = -1.0;

fn default_stop_all_fade() -> f64 {
    0.5
}
fn is_default_stop_all_fade(x: &f64) -> bool {
    *x == 0.5
}

fn default_group_fade() -> f64 {
    1.0
}
fn is_default_group_fade(x: &f64) -> bool {
    *x == 1.0
}

/// MIDI message kinds that can trigger things.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MidiKind {
    Note,
    Cc,
    Program,
}

/// A MIDI message that triggers something: note-on, a CC with a non-zero
/// value, or a program change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MidiTrigger {
    pub kind: MidiKind,
    /// Note, controller or program number (0-127).
    pub number: u8,
    /// 1-16, or `None` for any channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<u8>,
}

impl MidiTrigger {
    pub fn matches(&self, kind: MidiKind, number: u8, channel: u8) -> bool {
        self.kind == kind && self.number == number && self.channel.is_none_or(|c| c == channel)
    }

    pub fn describe(&self) -> String {
        let what = match self.kind {
            MidiKind::Note => format!("Note {}", self.number),
            MidiKind::Cc => format!("CC {}", self.number),
            MidiKind::Program => format!("Program {}", self.number),
        };
        match self.channel {
            Some(c) => format!("{what} ch {c}"),
            None => what,
        }
    }
}

/// Show-wide MIDI controls.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlsDoc {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_all: Option<MidiTrigger>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_bank: Option<MidiTrigger>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_bank: Option<MidiTrigger>,
    /// GO on the rundown: fire the standby step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rundown_go: Option<MidiTrigger>,
    /// Fade out the most recently fired pad still playing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_last: Option<MidiTrigger>,
}

/// One step of the game rundown: a pad to fire, or a note (e.g.
/// "Timeout 1") when `pad` is `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RundownItem {
    /// Unique within the rundown.
    pub id: u64,
    /// Slot of the pad this step fires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pad: Option<u32>,
    /// A note, or a label shown instead of the pad's title.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
}

impl Default for Show {
    fn default() -> Self {
        Self {
            format: SHOW_FORMAT.into(),
            version: SHOW_VERSION,
            name: "Untitled Show".into(),
            slots: default_slots(),
            banks: default_banks(),
            columns: default_columns(),
            outputs: default_outputs(),
            cues: Vec::new(),
            controls: ControlsDoc::default(),
            group_fade: default_group_fade(),
            stop_all_fade: default_stop_all_fade(),
            preview_output: None,
            level_match: None,
            rundown: Vec::new(),
            library: Vec::new(),
            pad_defaults: None,
            single_play: Vec::new(),
        }
    }
}

/// A device remembered by id, with its name as a fallback for when the id
/// changes (e.g. the same model of interface on another Mac).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRef {
    pub id: String,
    #[serde(default)]
    pub name: String,
}

/// A logical output and the devices it plays through.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutputDoc {
    pub name: String,
    /// Preferred device; `None` = the system default output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<DeviceRef>,
    /// Used automatically if the preferred device fails or is missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup: Option<DeviceRef>,
}

impl OutputDoc {
    pub fn named(name: &str) -> Self {
        Self {
            name: name.into(),
            device: None,
            backup: None,
        }
    }
}

/// One pad.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CueDoc {
    /// Grid position, `0..show.slots`.
    pub slot: u32,
    #[serde(default)]
    pub title: String,
    /// A single character that fires the cue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hotkey: Option<String>,
    /// Index into `Show::outputs`.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub output: u32,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub gain: f32,
    /// Loop the whole cue forever.
    #[serde(default, rename = "loop", skip_serializing_if = "is_false")]
    pub looping: bool,
    /// Display colour (UI only): a name ("blue") or "#RRGGBB".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Picture shown on the pad (UI only); found like audio files, and in
    /// an `Images` folder beside the show.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default)]
    pub clips: Vec<ClipDoc>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<MarkerDoc>,
    /// MIDI message that fires the cue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub midi: Option<MidiTrigger>,
    /// Exclusive group: firing this cue fades out other playing cues with
    /// the same group name (e.g. "Music").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// While playing, lower every other cue on the same output by this
    /// many dB (negative, e.g. -12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duck: Option<f32>,
    #[serde(default, skip_serializing_if = "is_default_retrigger")]
    pub retrigger: Retrigger,
    /// Output channels; `None` = all channels of the output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<RouteDoc>,
    /// Fade in over this many seconds each time the pad is fired.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub fade_in: f64,
    /// Length of this pad's fade-outs (Fade Out, "pressed again: fade",
    /// OSC fade); `None` = 0.5 s.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_out: Option<f64>,
    /// Makes the pad a playlist: each press (or the end of each song, with
    /// auto-advance) plays the next item. `clips` and `markers` are not
    /// used by playlist pads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playlist: Option<PlaylistDoc>,
    /// Sponsor credited when this pad plays (for sponsor reports).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sponsor: Option<String>,
    /// Follow the show's level matching (when it is on). Off keeps this
    /// pad's files exactly as they are, e.g. a deliberately quiet bed.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub level_match: bool,
    /// How this pad treats its bank's Multiplay setting.
    #[serde(default, skip_serializing_if = "is_default_play_mode")]
    pub play_mode: PlayMode,
    /// Actions run, in order, each time the pad starts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_fire: Vec<FireAction>,
}

/// How a pad treats its bank's Multiplay setting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayMode {
    /// As the bank says.
    #[default]
    Bank,
    /// Plays over everything: never fades other pads out, and isn't faded
    /// out when another pad in a single-play bank fires (horns, stings).
    Over,
    /// Fades out the bank's other pads when fired, even in a Multiplay
    /// bank.
    Solo,
}

fn is_default_play_mode(m: &PlayMode) -> bool {
    *m == PlayMode::Bank
}

/// What a pad action does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FireActionKind {
    /// Stop every other playing pad (or only this bank's, with
    /// `this_bank`), over `seconds` (default: the Stop All fade).
    StopOthers,
    /// Fire `pad`, after `seconds` (default now).
    FirePad,
    /// Stop `pad` over `seconds` (default: its own fade-out).
    StopPad,
    /// Lower every other pad by `db` for `seconds`, then bring it back.
    Duck,
    /// Show bank `bank` (0-based).
    Bank,
}

/// One action a pad runs when it starts. Flat, like [`ActionDoc`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FireAction {
    pub kind: FireActionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pad: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bank: Option<u32>,
    /// StopOthers: only pads in the same bank.
    #[serde(default, skip_serializing_if = "is_false")]
    pub this_bank: bool,
}

impl FireAction {
    pub fn new(kind: FireActionKind) -> Self {
        Self {
            kind,
            pad: None,
            seconds: None,
            db: None,
            bank: None,
            this_bank: false,
        }
    }

    /// The pad this action fires or stops, if any.
    pub fn target_pad(&self) -> Option<u32> {
        match self.kind {
            FireActionKind::FirePad | FireActionKind::StopPad => self.pad,
            _ => None,
        }
    }
}

/// Default pad fade-out when none is set, seconds.
pub const DEFAULT_FADE_OUT: f64 = 0.5;

/// A playlist pad's songs and how it moves through them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaylistDoc {
    /// Audio files, in play order (unless shuffled).
    pub items: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub shuffle: bool,
    /// Carry on into the next item when one ends (otherwise each press
    /// plays one item).
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub auto_advance: bool,
    /// Crossfade between items, seconds.
    #[serde(
        default = "default_crossfade",
        skip_serializing_if = "is_default_crossfade"
    )]
    pub crossfade: f64,
}

fn yes() -> bool {
    true
}
fn is_true(b: &bool) -> bool {
    *b
}
fn default_crossfade() -> f64 {
    2.0
}
fn is_default_crossfade(x: &f64) -> bool {
    *x == 2.0
}

impl PlaylistDoc {
    pub fn new(items: Vec<String>) -> Self {
        Self {
            items,
            shuffle: false,
            auto_advance: true,
            crossfade: default_crossfade(),
        }
    }
}

impl CueDoc {
    /// A pad playing one file.
    pub fn simple(slot: u32, title: &str, file: &str) -> Self {
        Self {
            slot,
            title: title.into(),
            hotkey: None,
            output: 0,
            gain: 1.0,
            looping: false,
            color: None,
            image: None,
            clips: vec![ClipDoc::file(file)],
            markers: Vec::new(),
            midi: None,
            group: None,
            duck: None,
            retrigger: Retrigger::Restart,
            route: None,
            fade_in: 0.0,
            fade_out: None,
            playlist: None,
            sponsor: None,
            level_match: true,
            play_mode: PlayMode::Bank,
            on_fire: Vec::new(),
        }
    }

    /// Every audio file the cue uses.
    pub fn files(&self) -> Vec<String> {
        let mut files: Vec<String> = self.clips.iter().map(|c| c.file.clone()).collect();
        if let Some(playlist) = &self.playlist {
            files.extend(playlist.items.iter().cloned());
        }
        files
    }

    /// The pad's fade-out length, seconds.
    pub fn fade_out_seconds(&self) -> f64 {
        self.fade_out.unwrap_or(DEFAULT_FADE_OUT).max(0.0)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurveDoc {
    #[default]
    Linear,
    EqualPower,
}

/// A region of a file placed on a cue's timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipDoc {
    pub file: String,
    /// Position on the cue's timeline.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub start: f64,
    /// Where in the file to begin.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub offset: f64,
    /// How much to play; `None` = to the end of the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<f64>,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub gain: f32,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub fade_in: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub fade_out: f64,
    #[serde(default)]
    pub fade_curve: CurveDoc,
}

impl ClipDoc {
    pub fn file(file: &str) -> Self {
        Self {
            file: file.into(),
            start: 0.0,
            offset: 0.0,
            length: None,
            gain: 1.0,
            fade_in: 0.0,
            fade_out: 0.0,
            fade_curve: CurveDoc::Linear,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Jump,
    Loop,
    Pause,
    Stop,
    FadeOut,
    /// Fire another pad (`pad`), as if pressed.
    Fire,
    /// Stop another pad (`pad`) over `seconds` (default: its fade-out).
    StopPad,
    /// Ramp this pad's volume to `db` over `seconds` (default 0 s).
    Volume,
}

/// Where a jump or loop goes: a time, or another marker's position.
/// Exactly one should be set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct TargetDoc {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker: Option<u64>,
}

/// What a marker does. Flat (rather than an enum with payloads) so it maps
/// directly onto a Swift `Codable` struct.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ActionDoc {
    pub kind: ActionKind,
    /// Jump/Loop target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<TargetDoc>,
    /// Loop: times to jump back; `None` = forever.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    /// FadeOut: duration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
    /// Loop: repeat until the operator presses the pad again (a vamp),
    /// then continue past this marker. Overrides `count`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hold: bool,
    /// Fire/StopPad: the target pad's slot (`bank * slots + pad`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pad: Option<u32>,
    /// Volume: level in dB (0 = as set on the pad, negative = quieter).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db: Option<f32>,
}

impl ActionDoc {
    pub fn new(kind: ActionKind) -> Self {
        Self {
            kind,
            to: None,
            count: None,
            seconds: None,
            hold: false,
            pad: None,
            db: None,
        }
    }

    /// The pad this action fires or stops, if any.
    pub fn target_pad(&self) -> Option<u32> {
        match self.kind {
            ActionKind::Fire | ActionKind::StopPad => self.pad,
            _ => None,
        }
    }
}

/// How a marker can reach the pad it fires or stops.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PadTarget {
    /// No cue there (or no such slot).
    Empty,
    /// A plain pad on the same output: the engine acts on it at the exact
    /// sample. Its fade times are baked into the marker.
    Local { fade_in: f64, fade_out: f64 },
    /// On another output, or a playlist: the session acts on it when it
    /// sees the marker reached (within one poll).
    Elsewhere,
}

/// What pressing a pad does while it is already playing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Retrigger {
    /// Play again from the start.
    #[default]
    Restart,
    /// Stop (with a click-free micro-fade).
    Stop,
    /// Fade out.
    Fade,
}

fn is_default_retrigger(r: &Retrigger) -> bool {
    *r == Retrigger::Restart
}

/// Output channels a cue plays through, 1-based (e.g. 3 and 4 of an
/// 8-channel interface: `first` 3, `count` 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteDoc {
    pub first: u16,
    pub count: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarkerDoc {
    /// Unique within the cue.
    pub id: u64,
    #[serde(default)]
    pub name: String,
    pub time: f64,
    pub action: ActionDoc,
}

/// Why a show could not be read.
#[derive(Debug)]
pub enum ShowError {
    Io(std::io::Error),
    Parse(String),
    NotAShow,
    /// Written by a newer version of the app.
    TooNew(u32),
    Invalid(String),
}

impl fmt::Display for ShowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "could not read show: {e}"),
            Self::Parse(e) => write!(f, "show file is damaged: {e}"),
            Self::NotAShow => write!(f, "not a Sportsball Jukebox show file"),
            Self::TooNew(v) => write!(
                f,
                "show was saved by a newer version (format {v}); please update the app"
            ),
            Self::Invalid(why) => write!(f, "invalid show: {why}"),
        }
    }
}

impl std::error::Error for ShowError {}

impl Show {
    pub fn from_json(json: &str) -> Result<Self, ShowError> {
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|e| ShowError::Parse(e.to_string()))?;
        if value.get("format").and_then(|f| f.as_str()) != Some(SHOW_FORMAT) {
            return Err(ShowError::NotAShow);
        }
        let version = value.get("version").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        if version > SHOW_VERSION {
            return Err(ShowError::TooNew(version));
        }
        // Migrations from older versions go here.
        let show: Show =
            serde_json::from_value(value).map_err(|e| ShowError::Parse(e.to_string()))?;
        show.validate()?;
        Ok(show)
    }

    pub fn to_json(&self) -> String {
        // Serialising these types cannot fail (no maps with non-string keys).
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn load(path: &Path) -> Result<Self, ShowError> {
        let json = std::fs::read_to_string(path).map_err(ShowError::Io)?;
        Self::from_json(&json)
    }

    /// Writes atomically (temp file + rename) so a crash mid-save never
    /// leaves a half-written show.
    pub fn save(&self, path: &Path) -> Result<(), ShowError> {
        let tmp = path.with_extension("sbjshow.tmp");
        std::fs::write(&tmp, self.to_json()).map_err(ShowError::Io)?;
        std::fs::rename(&tmp, path).map_err(ShowError::Io)
    }

    /// Structural checks. Problems that only affect one cue (missing
    /// files, bad marker targets) are reported per cue when loading rather
    /// than rejecting the whole show.
    pub fn validate(&self) -> Result<(), ShowError> {
        if self.outputs.is_empty() {
            return Err(ShowError::Invalid(
                "a show needs at least one output".into(),
            ));
        }
        if self.slots == 0 || self.slots > MAX_PADS_PER_BANK {
            return Err(ShowError::Invalid(format!(
                "pads per bank must be 1-{MAX_PADS_PER_BANK}"
            )));
        }
        if self.banks.is_empty() || self.total_slots() > MAX_SLOTS {
            return Err(ShowError::Invalid(format!(
                "a show needs 1 or more banks and at most {MAX_SLOTS} pads in total"
            )));
        }
        let mut seen = std::collections::HashSet::new();
        for cue in &self.cues {
            if cue.slot >= self.total_slots() {
                return Err(ShowError::Invalid(format!(
                    "cue \"{}\" is in pad {} but the show has {} pads",
                    cue.title,
                    cue.slot + 1,
                    self.total_slots()
                )));
            }
            if !seen.insert(cue.slot) {
                return Err(ShowError::Invalid(format!(
                    "two cues in pad {}",
                    cue.slot + 1
                )));
            }
        }
        Ok(())
    }

    /// Pads across all banks.
    pub fn total_slots(&self) -> u32 {
        self.slots.saturating_mul(self.banks.len() as u32)
    }

    pub fn cue(&self, slot: u32) -> Option<&CueDoc> {
        self.cues.iter().find(|c| c.slot == slot)
    }

    /// Every file referenced by any cue.
    pub fn files(&self) -> Vec<String> {
        let mut files: Vec<String> = self.cues.iter().flat_map(CueDoc::files).collect();
        files.sort();
        files.dedup();
        files
    }

    /// The library: files kept with the show plus every file a pad uses.
    pub fn library_files(&self) -> Vec<String> {
        let mut files = self.files();
        files.extend(self.library.iter().cloned());
        files.sort();
        files.dedup();
        files
    }
}

/// Finds a referenced file, trying the stored path first and then, if the
/// show has moved (e.g. copied to another Mac with its audio folder), the
/// same file name next to the show or in an `Audio` (or, for pad
/// pictures, `Images`) folder beside it.
pub fn resolve_file(file: &str, show_dir: Option<&Path>) -> Option<PathBuf> {
    let path = PathBuf::from(file);
    if path.is_file() {
        return Some(path);
    }
    let dir = show_dir?;
    if path.is_relative() {
        let candidate = dir.join(&path);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let name = path.file_name()?;
    [
        dir.join(name),
        dir.join("Audio").join(name),
        dir.join("Images").join(name),
    ]
    .into_iter()
    .find(|p| p.is_file())
}

/// A problem with one cue, reported to the operator but not fatal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CueProblem {
    pub slot: u32,
    pub message: String,
}

/// Builds the engine timeline for a cue at `rate`. `audio` maps file
/// strings (as written in the show) to decoded audio at `rate`; clips
/// whose audio is missing are skipped and reported.
pub fn cue_timeline(
    cue: &CueDoc,
    rate: u32,
    audio: &HashMap<String, Arc<AudioBuffer>>,
    pads: &dyn Fn(u32) -> PadTarget,
    level: &dyn Fn(&str) -> f32,
) -> (Timeline, Vec<CueProblem>) {
    let mut problems = Vec::new();
    let mut problem = |message: String| {
        problems.push(CueProblem {
            slot: cue.slot,
            message,
        })
    };
    let secs = |s: f64| seconds_to_samples(s, rate);

    let mut builder = Timeline::builder(rate);
    let mut content_end: Samples = 0;

    for clip in &cue.clips {
        let Some(buffer) = audio.get(&clip.file) else {
            problem(format!("{MISSING_FILE}{}", clip.file));
            continue;
        };
        let offset = secs(clip.offset).min(buffer.frames());
        let available = buffer.frames() - offset;
        let length = clip.length.map_or(available, |l| secs(l).min(available));
        if length == 0 {
            problem(format!("clip of {} is empty", clip.file));
            continue;
        }
        let curve = match clip.fade_curve {
            CurveDoc::Linear => FadeCurve::Linear,
            CurveDoc::EqualPower => FadeCurve::EqualPower,
        };
        let gain = if clip.gain.is_finite() {
            clip.gain.max(0.0)
        } else {
            1.0
        };
        let start = secs(clip.start);
        content_end = content_end.max(start + length);
        builder = builder.clip(
            Clip::new(Arc::clone(buffer))
                .at(start)
                .trimmed(offset, length)
                .with_gain(gain * level(&clip.file))
                .with_fade_in(Fade::new(secs(clip.fade_in).min(length), curve))
                .with_fade_out(Fade::new(secs(clip.fade_out).min(length), curve)),
        );
    }

    // Marker positions, for resolving marker-relative targets.
    let positions: HashMap<u64, Samples> =
        cue.markers.iter().map(|m| (m.id, secs(m.time))).collect();
    let mut last_marker: Samples = 0;

    for marker in &cue.markers {
        if marker.id == WHOLE_CUE_LOOP_ID {
            problem(format!("marker \"{}\" uses a reserved id", marker.name));
            continue;
        }
        let position = secs(marker.time);
        let target = |to: Option<TargetDoc>| -> Option<Samples> {
            let to = to?;
            match (to.marker, to.time) {
                (Some(id), _) => positions.get(&id).copied(),
                (None, Some(t)) => Some(secs(t)),
                (None, None) => None,
            }
        };
        let action = match marker.action.kind {
            ActionKind::Jump | ActionKind::Loop => {
                let Some(to) = target(marker.action.to) else {
                    problem(format!("marker \"{}\" has no valid target", marker.name));
                    continue;
                };
                if marker.action.kind == ActionKind::Jump {
                    MarkerAction::Jump { to }
                } else {
                    MarkerAction::Loop {
                        to,
                        count: if marker.action.hold {
                            LoopCount::UntilReleased
                        } else {
                            marker
                                .action
                                .count
                                .map_or(LoopCount::Infinite, LoopCount::Times)
                        },
                    }
                }
            }
            ActionKind::Pause => MarkerAction::Pause,
            ActionKind::Stop => MarkerAction::Stop,
            ActionKind::FadeOut => MarkerAction::FadeOut {
                length: secs(marker.action.seconds.unwrap_or(1.0)),
            },
            ActionKind::Fire | ActionKind::StopPad => {
                let Some(pad) = marker.action.pad else {
                    problem(format!("marker \"{}\" has no pad to act on", marker.name));
                    continue;
                };
                let player = sbj_engine::PlayerId(pad);
                match (pads(pad), marker.action.kind) {
                    (PadTarget::Empty, _) => {
                        problem(format!("marker \"{}\" acts on an empty pad", marker.name));
                        continue;
                    }
                    (PadTarget::Elsewhere, _) => MarkerAction::Notify,
                    (PadTarget::Local { fade_in, .. }, ActionKind::Fire) => MarkerAction::Start {
                        player,
                        fade_in: secs(fade_in),
                    },
                    (PadTarget::Local { fade_out, .. }, _) => MarkerAction::StopPlayer {
                        player,
                        fade: secs(marker.action.seconds.unwrap_or(fade_out)),
                    },
                }
            }
            ActionKind::Volume => {
                let db = marker.action.db.unwrap_or(0.0);
                let db = if db.is_finite() {
                    db.clamp(-96.0, 12.0)
                } else {
                    0.0
                };
                MarkerAction::Level {
                    gain: if db <= -96.0 {
                        0.0
                    } else {
                        10f32.powf(db / 20.0)
                    },
                    length: secs(marker.action.seconds.unwrap_or(0.0)),
                }
            }
        };
        last_marker = last_marker.max(position);
        builder = builder.marker(Marker {
            id: MarkerId(marker.id),
            position,
            action,
        });
    }

    let end = content_end.max(last_marker);
    if cue.looping && end > 0 {
        builder = builder.marker(Marker::new(
            WHOLE_CUE_LOOP_ID,
            end,
            MarkerAction::Loop {
                to: 0,
                count: LoopCount::Infinite,
            },
        ));
    }

    // Targets are resolved within the content, so the only remaining
    // failure is a target past the end; drop markers until it builds.
    let mut builder = builder.length(end);
    loop {
        match builder.clone().build() {
            Ok(timeline) => return (timeline, problems),
            Err(sbj_engine::TimelineError::TargetOutOfRange { marker, .. }) => {
                problem(format!("marker {} jumps past the end of the cue", marker.0));
                builder = builder.without_marker(marker);
            }
            Err(e) => {
                problem(e.to_string());
                let empty = Timeline::builder(rate.max(1))
                    .build()
                    .expect("an empty timeline always builds");
                return (empty, problems);
            }
        }
    }
}
