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
    /// Display colour name (UI only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
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
            clips: vec![ClipDoc::file(file)],
            markers: Vec::new(),
            midi: None,
            group: None,
            duck: None,
            retrigger: Retrigger::Restart,
            route: None,
        }
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
}

impl ActionDoc {
    pub fn new(kind: ActionKind) -> Self {
        Self {
            kind,
            to: None,
            count: None,
            seconds: None,
            hold: false,
        }
    }
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
        let mut files: Vec<String> = self
            .cues
            .iter()
            .flat_map(|c| c.clips.iter().map(|clip| clip.file.clone()))
            .collect();
        files.sort();
        files.dedup();
        files
    }
}

/// Finds a referenced file, trying the stored path first and then, if the
/// show has moved (e.g. copied to another Mac with its audio folder), the
/// same file name next to the show or in an `Audio` folder beside it.
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
    [dir.join(name), dir.join("Audio").join(name)]
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
                .with_gain(gain)
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
