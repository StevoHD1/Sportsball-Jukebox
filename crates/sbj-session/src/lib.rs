//! The app model: a show (pads, timelines, markers, outputs) playing
//! through one or more audio devices.
//!
//! * Each logical output in the show gets its own engine and device
//!   stream, so interfaces run independently on their own clocks.
//! * Each pad slot maps to the engine player with the same index, on
//!   whichever output the cue is routed to.
//! * If an output's device fails, [`Session::poll`] reopens it on the
//!   backup device (or the system default) and resumes what was playing.
//!
//! This is what the UI drives (through `sbj-ffi`). Slow work (decoding
//! files) can be done without holding the session, via [`decode_files`]
//! and [`Session::add_sound`].

pub mod show;

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sbj_backend::{
    AudioBackend, BackendError, DeviceInfo, OutputStream, StreamConfig, StreamHealth,
};
use sbj_engine::{
    AudioBuffer, Controller, Engine, EngineConfig, EngineError, PlayerId, samples_to_seconds,
    seconds_to_samples,
};
use sbj_media::MediaError;

pub mod control;
mod playback;
mod preflight;

pub use control::{ControlMessage, LearnTarget, MidiMessage};
pub use preflight::{Check, CheckLevel, Preflight};
pub use show::{
    ActionDoc, ActionKind, ClipDoc, ControlsDoc, CueDoc, CueProblem, CurveDoc, DeviceRef,
    MAX_PADS_PER_BANK, MAX_SLOTS, MarkerDoc, MidiKind, MidiTrigger, OutputDoc, Retrigger, RouteDoc,
    Show, ShowError, TargetDoc,
};

/// How often to retry an output whose devices are all unavailable.
pub const DEFAULT_RETRY_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub enum SessionError {
    InvalidCue(u32),
    InvalidOutput(u32),
    NoOutput,
    Backend(BackendError),
    Engine(EngineError),
    Media(MediaError),
    Show(ShowError),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCue(i) => write!(f, "pad {} does not exist", i + 1),
            Self::InvalidOutput(i) => write!(f, "output {} does not exist", i + 1),
            Self::NoOutput => write!(f, "the cue's audio output is not available"),
            Self::Backend(e) => e.fmt(f),
            Self::Engine(e) => e.fmt(f),
            Self::Media(e) => e.fmt(f),
            Self::Show(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for SessionError {}

impl From<BackendError> for SessionError {
    fn from(e: BackendError) -> Self {
        Self::Backend(e)
    }
}
impl From<EngineError> for SessionError {
    fn from(e: EngineError) -> Self {
        Self::Engine(e)
    }
}
impl From<MediaError> for SessionError {
    fn from(e: MediaError) -> Self {
        Self::Media(e)
    }
}
impl From<ShowError> for SessionError {
    fn from(e: ShowError) -> Self {
        Self::Show(e)
    }
}

pub type Result<T> = std::result::Result<T, SessionError>;

// ---------------------------------------------------------------------
// Sounds
// ---------------------------------------------------------------------

/// A decoded audio file, with copies converted to output sample rates.
#[derive(Debug, Clone)]
pub struct Sound {
    original: Arc<AudioBuffer>,
    by_rate: HashMap<u32, Arc<AudioBuffer>>,
}

impl Sound {
    /// Decodes `path` and pre-converts it to each of `rates`. Slow: call
    /// off the main thread.
    pub fn load(path: impl AsRef<Path>, rates: &[u32]) -> Result<Self> {
        Self::from_buffer(sbj_media::decode_file(path)?, rates)
    }

    pub fn from_buffer(buffer: AudioBuffer, rates: &[u32]) -> Result<Self> {
        let mut sound = Self {
            original: Arc::new(buffer),
            by_rate: HashMap::new(),
        };
        for &rate in rates {
            sound.at_rate(rate)?;
        }
        Ok(sound)
    }

    pub fn duration_seconds(&self) -> f64 {
        samples_to_seconds(self.original.frames(), self.original.sample_rate())
    }

    pub fn channels(&self) -> u16 {
        self.original.channels()
    }

    /// Memory used by this sound's decoded audio, in bytes.
    pub fn memory_bytes(&self) -> u64 {
        let samples: usize = std::iter::once(&self.original)
            .chain(self.by_rate.values())
            .map(|b| b.data().len())
            .sum();
        (samples * std::mem::size_of::<f32>()) as u64
    }

    pub fn original(&self) -> &Arc<AudioBuffer> {
        &self.original
    }

    /// The audio at `rate`, converting (slow) the first time.
    fn at_rate(&mut self, rate: u32) -> Result<Arc<AudioBuffer>> {
        if self.original.sample_rate() == rate {
            return Ok(Arc::clone(&self.original));
        }
        if let Some(b) = self.by_rate.get(&rate) {
            return Ok(Arc::clone(b));
        }
        let converted = Arc::new(sbj_media::resample(&self.original, rate)?);
        self.by_rate.insert(rate, Arc::clone(&converted));
        Ok(converted)
    }
}

/// Result of decoding a batch of files.
#[derive(Debug, Default)]
pub struct DecodeReport {
    /// Keyed by the file string as written in the show.
    pub sounds: Vec<(String, Sound)>,
    /// File and reason.
    pub failures: Vec<(String, String)>,
}

/// Decodes files in parallel, without touching any session. `show_dir`
/// lets moved shows find their audio (see [`show::resolve_file`]).
pub fn decode_files(files: &[String], show_dir: Option<&Path>, rates: &[u32]) -> DecodeReport {
    let workers = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .clamp(1, 8);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results = std::sync::Mutex::new(DecodeReport::default());

    std::thread::scope(|scope| {
        for _ in 0..workers.min(files.len()) {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(file) = files.get(i) else { break };
                    let outcome = match show::resolve_file(file, show_dir) {
                        None => Err("file not found".to_string()),
                        Some(path) => Sound::load(&path, rates).map_err(|e| e.to_string()),
                    };
                    let mut report = results.lock().unwrap_or_else(|e| e.into_inner());
                    match outcome {
                        Ok(sound) => report.sounds.push((file.clone(), sound)),
                        Err(why) => report.failures.push((file.clone(), why)),
                    }
                }
            });
        }
    });
    results.into_inner().unwrap_or_else(|e| e.into_inner())
}

/// Min/max peaks of `buffer` (all channels mixed) in `buckets` equal
/// slices, for drawing waveforms.
pub fn waveform(buffer: &AudioBuffer, buckets: usize) -> Vec<(f32, f32)> {
    let frames = buffer.frames() as usize;
    let ch = usize::from(buffer.channels());
    if buckets == 0 || frames == 0 {
        return Vec::new();
    }
    let data = buffer.data();
    (0..buckets)
        .map(|b| {
            let from = b * frames / buckets;
            let to = ((b + 1) * frames / buckets).max(from + 1).min(frames);
            let mut lo = 0.0f32;
            let mut hi = 0.0f32;
            for frame in data[from * ch..to * ch].chunks_exact(ch) {
                for &s in frame {
                    lo = lo.min(s);
                    hi = hi.max(s);
                }
            }
            (lo, hi)
        })
        .collect()
}

// ---------------------------------------------------------------------
// State snapshots for the UI
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CueState {
    pub assigned: bool,
    pub playing: bool,
    pub looping: bool,
    /// The cue's output is open and the cue is loaded on it.
    pub ready: bool,
    /// Playing a vamp: pressing the pad will let it continue.
    pub holding: bool,
    pub position_seconds: f64,
    pub duration_seconds: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputStatus {
    /// Playing through its preferred device (or the system default).
    Running,
    /// Playing through its backup because the preferred device is missing
    /// or failed.
    OnBackup,
    /// No device available; retrying.
    Offline,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputState {
    pub name: String,
    pub status: OutputStatus,
    pub device_id: Option<String>,
    pub device_name: Option<String>,
    pub sample_rate: u32,
    pub channels: u16,
    /// Device-reported glitches since this device was opened.
    pub xruns: u64,
    pub error: Option<String>,
}

// ---------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------

struct Live<S> {
    device: DeviceInfo,
    config: StreamConfig,
    controller: Controller,
    stream: S,
}

struct OutputRuntime<S> {
    live: Option<Live<S>>,
    on_backup: bool,
    error: Option<String>,
    retry_at: Option<Instant>,
}

impl<S> Default for OutputRuntime<S> {
    fn default() -> Self {
        Self {
            live: None,
            on_backup: false,
            error: None,
            retry_at: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct LoadedCue {
    output: usize,
    length_seconds: f64,
}

pub struct Session<B: AudioBackend> {
    backend: B,
    show: Show,
    show_path: Option<PathBuf>,
    dirty: bool,
    library: HashMap<String, Sound>,
    file_errors: HashMap<String, String>,
    outputs: Vec<OutputRuntime<B::Stream>>,
    loaded: HashMap<u32, LoadedCue>,
    problems: Vec<CueProblem>,
    notices: Vec<String>,
    master_gain: f32,
    retry_interval: Duration,
    /// Override for the engine's declick length (tests use 0).
    declick_seconds: Option<f64>,
    current_bank: u32,
    learning: Option<LearnTarget>,
    last_control: Option<String>,
    /// Duck factor (linear) currently applied to each slot.
    duck_applied: HashMap<u32, f32>,
    /// When each slot was last fired, so a cue counts as playing before
    /// the audio thread has started it.
    recent_fires: HashMap<u32, Instant>,
}

impl<B: AudioBackend> Session<B> {
    /// A session with an empty default show. No device is opened until
    /// [`Session::open_outputs`].
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            show: Show::default(),
            show_path: None,
            dirty: false,
            library: HashMap::new(),
            file_errors: HashMap::new(),
            outputs: vec![OutputRuntime::default()],
            loaded: HashMap::new(),
            problems: Vec::new(),
            notices: Vec::new(),
            master_gain: 1.0,
            retry_interval: DEFAULT_RETRY_INTERVAL,
            declick_seconds: None,
            current_bank: 0,
            learning: None,
            last_control: None,
            duck_applied: HashMap::new(),
            recent_fires: HashMap::new(),
        }
    }

    /// Overrides the length of the click-removing crossfades and stop
    /// fades (0 = hard cuts). Takes effect when outputs are next opened.
    pub fn set_declick(&mut self, seconds: f64) {
        self.declick_seconds = Some(seconds.max(0.0));
    }

    /// How often to retry an output whose devices are all unavailable.
    pub fn set_retry_interval(&mut self, interval: Duration) {
        self.retry_interval = interval;
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn devices(&self) -> Result<Vec<DeviceInfo>> {
        Ok(self.backend.output_devices()?)
    }

    // ---- Show document ------------------------------------------------

    pub fn show(&self) -> &Show {
        &self.show
    }

    pub fn show_path(&self) -> Option<&Path> {
        self.show_path.as_deref()
    }

    /// Unsaved changes since the last load or save.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Replaces the show. Everything stops. `sounds` should hold the
    /// decoded audio for the show's files (see [`decode_files`]); any
    /// missing files are reported as cue problems.
    pub fn install_show(&mut self, show: Show, path: Option<PathBuf>, decoded: DecodeReport) {
        self.close_outputs();
        self.library.clear();
        self.file_errors.clear();
        self.add_decoded(decoded);
        self.show = show;
        self.show_path = path;
        self.current_bank = 0;
        self.learning = None;
        self.dirty = false;
        self.outputs = (0..self.show.outputs.len())
            .map(|_| OutputRuntime::default())
            .collect();
        self.open_outputs();
    }

    pub fn new_show(&mut self) {
        self.install_show(Show::default(), None, DecodeReport::default());
    }

    pub fn save_show(&mut self, path: &Path) -> Result<()> {
        self.show.save(path)?;
        self.show_path = Some(path.to_path_buf());
        self.dirty = false;
        Ok(())
    }

    fn show_dir(&self) -> Option<PathBuf> {
        self.show_path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
    }

    /// Renames the show and sets the grid: pads per bank, columns and bank
    /// names. Cues keep their position within their bank; shrinking is
    /// refused while cues occupy pads or banks that would be removed.
    pub fn set_show_settings(
        &mut self,
        name: &str,
        slots: u32,
        columns: u32,
        banks: Vec<String>,
    ) -> Result<()> {
        let invalid = |m: String| Err(SessionError::Show(ShowError::Invalid(m)));
        if slots == 0 || slots > MAX_PADS_PER_BANK {
            return invalid(format!("pads per bank must be 1-{MAX_PADS_PER_BANK}"));
        }
        if banks.is_empty() || slots * banks.len() as u32 > MAX_SLOTS {
            return invalid(format!(
                "at least one bank, and at most {MAX_SLOTS} pads in total"
            ));
        }
        let old_slots = self.show.slots;
        for cue in &self.show.cues {
            let (bank, pad) = (cue.slot / old_slots, cue.slot % old_slots);
            if pad >= slots || bank as usize >= banks.len() {
                return invalid(format!(
                    "\"{}\" (bank {}, pad {}) is in a pad that would be removed",
                    cue.title,
                    bank + 1,
                    pad + 1
                ));
            }
        }
        // Renumber cues if the bank size changed, keeping bank and pad.
        let mut moved = Vec::new();
        if slots != old_slots {
            for cue in &mut self.show.cues {
                let (bank, pad) = (cue.slot / old_slots, cue.slot % old_slots);
                moved.push(cue.slot);
                cue.slot = bank * slots + pad;
                moved.push(cue.slot);
            }
        }
        self.show.name = name.to_string();
        self.show.slots = slots;
        self.show.columns = columns.clamp(1, 16);
        self.show.banks = banks;
        self.current_bank = self.current_bank.min(self.show.banks.len() as u32 - 1);
        self.dirty = true;
        moved.sort_unstable();
        moved.dedup();
        for slot in moved {
            self.load_cue(slot);
        }
        Ok(())
    }

    // ---- Banks ------------------------------------------------------------

    pub fn current_bank(&self) -> u32 {
        self.current_bank
    }

    pub fn set_bank(&mut self, bank: u32) -> Result<()> {
        if bank as usize >= self.show.banks.len() {
            return Err(SessionError::Show(ShowError::Invalid(format!(
                "bank {} does not exist",
                bank + 1
            ))));
        }
        self.current_bank = bank;
        Ok(())
    }

    /// Moves to the next (or previous) bank, wrapping around.
    pub fn step_bank(&mut self, forward: bool) {
        let count = self.show.banks.len() as u32;
        self.current_bank = if forward {
            (self.current_bank + 1) % count
        } else {
            (self.current_bank + count - 1) % count
        };
    }

    // ---- Sounds -------------------------------------------------------

    /// Sample rates of the open outputs; decode with these to keep
    /// resampling off the session.
    pub fn output_rates(&self) -> Vec<u32> {
        let mut rates: Vec<u32> = self
            .outputs
            .iter()
            .filter_map(|o| o.live.as_ref().map(|l| l.config.sample_rate))
            .collect();
        rates.sort_unstable();
        rates.dedup();
        rates
    }

    pub fn add_sound(&mut self, file: String, sound: Sound) {
        self.file_errors.remove(&file);
        self.library.insert(file, sound);
    }

    fn add_decoded(&mut self, decoded: DecodeReport) {
        for (file, sound) in decoded.sounds {
            self.add_sound(file, sound);
        }
        for (file, why) in decoded.failures {
            self.file_errors.insert(file, why);
        }
    }

    pub fn has_sound(&self, file: &str) -> bool {
        self.library.contains_key(file)
    }

    /// The decoded audio for `file`, at its original rate.
    pub fn sound_buffer(&self, file: &str) -> Option<Arc<AudioBuffer>> {
        self.library.get(file).map(|s| Arc::clone(s.original()))
    }

    // ---- Outputs ------------------------------------------------------

    /// Opens every output's device and loads its cues.
    pub fn open_outputs(&mut self) {
        for index in 0..self.outputs.len() {
            self.open_output(index);
        }
    }

    pub fn close_outputs(&mut self) {
        for output in &mut self.outputs {
            *output = OutputRuntime::default();
        }
        self.loaded.clear();
    }

    /// Replaces the output list. Outputs whose settings changed are
    /// reopened; cues routed to removed outputs move to the first.
    pub fn set_outputs(&mut self, outputs: Vec<OutputDoc>) -> Result<()> {
        if outputs.is_empty() {
            return Err(SessionError::Show(ShowError::Invalid(
                "a show needs at least one output".into(),
            )));
        }
        let old = std::mem::replace(&mut self.show.outputs, outputs);
        let count = self.show.outputs.len();
        for cue in &mut self.show.cues {
            if cue.output as usize >= count {
                cue.output = 0;
            }
        }
        self.outputs.truncate(count);
        self.outputs.resize_with(count, OutputRuntime::default);
        self.loaded.retain(|_, l| l.output < count);
        for index in 0..count {
            let changed = old.get(index) != Some(&self.show.outputs[index]);
            if changed || self.outputs[index].live.is_none() {
                self.open_output(index);
            }
        }
        // Cues moved off removed outputs need loading on their new one.
        let slots: Vec<u32> = self.show.cues.iter().map(|c| c.slot).collect();
        for slot in slots {
            if !self.loaded.contains_key(&slot) {
                self.load_cue(slot);
            }
        }
        self.dirty = true;
        Ok(())
    }

    /// Devices to try for an output, in order, with whether each counts
    /// as the backup.
    fn candidates(&self, doc: &OutputDoc) -> Vec<(DeviceInfo, bool)> {
        let devices = self.backend.output_devices().unwrap_or_default();
        let find = |r: &DeviceRef| {
            devices
                .iter()
                .find(|d| d.id.0 == r.id)
                .or_else(|| {
                    devices
                        .iter()
                        .find(|d| !r.name.is_empty() && d.name == r.name)
                })
                .cloned()
        };
        let default = devices.iter().find(|d| d.is_default).cloned();
        let mut list = Vec::new();
        match &doc.device {
            Some(r) => {
                if let Some(d) = find(r) {
                    list.push((d, false));
                }
            }
            None => {
                if let Some(d) = default.clone() {
                    list.push((d, false));
                }
            }
        }
        if let Some(d) = doc.backup.as_ref().and_then(find) {
            list.push((d, true));
        }
        if doc.device.is_some()
            && let Some(d) = default
        {
            list.push((d, true));
        }
        let mut seen = std::collections::HashSet::new();
        list.retain(|(d, _)| seen.insert(d.id.clone()));
        list
    }

    fn start_stream(&self, device: &DeviceInfo) -> Result<Live<B::Stream>> {
        let config = self.backend.preferred_config(&device.id)?;
        let mut engine = EngineConfig::new(config.sample_rate, config.channels);
        if let Some(seconds) = self.declick_seconds {
            engine.declick_frames = seconds_to_samples(seconds, config.sample_rate) as u32;
        }
        engine.players = MAX_SLOTS;
        let (mut controller, mut renderer) = Engine::create(engine)?;
        controller.set_master_gain(self.master_gain)?;
        let stream = self.backend.open_output(
            &device.id,
            config,
            Box::new(move |out| renderer.process(out)),
        )?;
        Ok(Live {
            device: device.clone(),
            config,
            controller,
            stream,
        })
    }

    /// (Re)opens one output on the best available device and loads its
    /// cues. Returns whether a device was opened.
    fn open_output(&mut self, index: usize) -> bool {
        self.outputs[index] = OutputRuntime::default();
        self.loaded.retain(|_, l| l.output != index);

        let doc = self.show.outputs[index].clone();
        let mut last_error = None;
        for (device, is_backup) in self.candidates(&doc) {
            match self.start_stream(&device) {
                Ok(live) => {
                    let output = &mut self.outputs[index];
                    output.live = Some(live);
                    output.on_backup = is_backup;
                    if is_backup {
                        self.notices.push(format!(
                            "Output \"{}\" is using backup device \"{}\".",
                            doc.name, device.name
                        ));
                    }
                    let slots: Vec<u32> = self
                        .show
                        .cues
                        .iter()
                        .filter(|c| self.output_index(c) == index)
                        .map(|c| c.slot)
                        .collect();
                    for slot in slots {
                        self.load_cue(slot);
                    }
                    return true;
                }
                Err(e) => last_error = Some(format!("{}: {e}", device.name)),
            }
        }
        let output = &mut self.outputs[index];
        output.error = Some(last_error.unwrap_or_else(|| "no audio device available".into()));
        output.retry_at = Some(Instant::now() + self.retry_interval);
        false
    }

    fn output_index(&self, cue: &CueDoc) -> usize {
        (cue.output as usize).min(self.outputs.len() - 1)
    }

    pub fn outputs_state(&self) -> Vec<OutputState> {
        self.show
            .outputs
            .iter()
            .zip(&self.outputs)
            .map(|(doc, rt)| match &rt.live {
                Some(live) => OutputState {
                    name: doc.name.clone(),
                    status: if rt.on_backup {
                        OutputStatus::OnBackup
                    } else {
                        OutputStatus::Running
                    },
                    device_id: Some(live.device.id.0.clone()),
                    device_name: Some(live.device.name.clone()),
                    sample_rate: live.config.sample_rate,
                    channels: live.config.channels,
                    xruns: live.stream.status().xruns(),
                    error: None,
                },
                None => OutputState {
                    name: doc.name.clone(),
                    status: OutputStatus::Offline,
                    device_id: None,
                    device_name: None,
                    sample_rate: 0,
                    channels: 0,
                    xruns: 0,
                    error: rt.error.clone(),
                },
            })
            .collect()
    }

    /// The live stream of output `index`, e.g. to pull audio from an
    /// offline backend in tests.
    pub fn output_stream_mut(&mut self, index: usize) -> Option<&mut B::Stream> {
        self.outputs
            .get_mut(index)
            .and_then(|o| o.live.as_mut())
            .map(|l| &mut l.stream)
    }

    // ---- Cues ---------------------------------------------------------

    /// Builds and loads a cue onto its output, replacing what the slot
    /// held. Problems (missing files etc.) are recorded, not returned.
    fn load_cue(&mut self, slot: u32) {
        self.problems.retain(|p| p.slot != slot);
        if let Some(previous) = self.loaded.remove(&slot)
            && let Some(live) = self.outputs[previous.output].live.as_mut()
        {
            let _ = live.controller.unload(PlayerId(slot));
        }
        let Some(cue) = self.show.cue(slot).cloned() else {
            return;
        };
        let index = self.output_index(&cue);
        if cue.output as usize != index {
            self.problems.push(CueProblem {
                slot,
                message: format!("output {} does not exist; using the first", cue.output + 1),
            });
        }
        let Some(live) = self.outputs[index].live.as_mut() else {
            return;
        };
        let rate = live.config.sample_rate;

        let mut audio = HashMap::new();
        for clip in &cue.clips {
            if let Some(sound) = self.library.get_mut(&clip.file) {
                match sound.at_rate(rate) {
                    Ok(buffer) => {
                        audio.insert(clip.file.clone(), buffer);
                    }
                    Err(e) => self.problems.push(CueProblem {
                        slot,
                        message: e.to_string(),
                    }),
                }
            }
        }
        let (timeline, problems) = show::cue_timeline(&cue, rate, &audio);
        for mut problem in problems {
            // Say why a file is missing when we know.
            if let Some(file) = problem.message.strip_prefix(show::MISSING_FILE)
                && let Some(why) = self.file_errors.get(file)
            {
                problem.message = format!("{file}: {why}");
            }
            self.problems.push(problem);
        }
        let length_seconds = samples_to_seconds(timeline.length(), rate);
        let player = PlayerId(slot);
        let channels = live.config.channels;
        let (first, count) = match cue.route {
            None => (0, 0),
            Some(r) if r.first >= 1 && r.count >= 1 && r.first - 1 + r.count <= channels => {
                (r.first - 1, r.count)
            }
            Some(r) => {
                self.problems.push(CueProblem {
                    slot,
                    message: format!(
                        "channels {}-{} are not available on this device ({channels} channels); playing on all channels",
                        r.first,
                        r.first + r.count.saturating_sub(1)
                    ),
                });
                (0, 0)
            }
        };
        let duck = self.duck_applied.get(&slot).copied().unwrap_or(1.0);
        let loaded = live
            .controller
            .load(player, timeline)
            .and_then(|()| live.controller.set_route(player, first, count))
            .and_then(|()| live.controller.set_player_gain(player, cue.gain * duck));
        match loaded {
            Ok(()) => {
                self.loaded.insert(
                    slot,
                    LoadedCue {
                        output: index,
                        length_seconds,
                    },
                );
            }
            Err(e) => self.problems.push(CueProblem {
                slot,
                message: e.to_string(),
            }),
        }
    }

    /// Adds or replaces the cue in `cue.slot` and loads it. Any audio it
    /// uses that is not yet decoded is decoded now (slow); to avoid that,
    /// decode first with [`decode_files`] and [`Session::add_sound`].
    pub fn set_cue(&mut self, cue: CueDoc) -> Result<()> {
        if cue.slot >= self.show.total_slots() {
            return Err(SessionError::InvalidCue(cue.slot));
        }
        if cue.output as usize >= self.show.outputs.len() {
            return Err(SessionError::InvalidOutput(cue.output));
        }
        let missing: Vec<String> = cue
            .clips
            .iter()
            .map(|c| c.file.clone())
            .filter(|f| !self.library.contains_key(f))
            .collect();
        if !missing.is_empty() {
            let dir = self.show_dir();
            let rates = self.output_rates();
            self.add_decoded(decode_files(&missing, dir.as_deref(), &rates));
        }
        let slot = cue.slot;
        match self.show.cues.iter_mut().find(|c| c.slot == slot) {
            Some(existing) => *existing = cue,
            None => {
                self.show.cues.push(cue);
                self.show.cues.sort_by_key(|c| c.slot);
            }
        }
        self.dirty = true;
        self.load_cue(slot);
        Ok(())
    }

    pub fn remove_cue(&mut self, slot: u32) -> Result<()> {
        if slot >= self.show.total_slots() {
            return Err(SessionError::InvalidCue(slot));
        }
        self.show.cues.retain(|c| c.slot != slot);
        self.dirty = true;
        self.load_cue(slot);
        Ok(())
    }

    /// Moves a cue to another pad, swapping with whatever is there.
    pub fn move_cue(&mut self, from: u32, to: u32) -> Result<()> {
        for s in [from, to] {
            if s >= self.show.total_slots() {
                return Err(SessionError::InvalidCue(s));
            }
        }
        for cue in &mut self.show.cues {
            if cue.slot == from {
                cue.slot = to;
            } else if cue.slot == to {
                cue.slot = from;
            }
        }
        self.show.cues.sort_by_key(|c| c.slot);
        self.dirty = true;
        self.load_cue(from);
        self.load_cue(to);
        Ok(())
    }

    fn live_for(&mut self, slot: u32) -> Result<(&mut Live<B::Stream>, LoadedCue)> {
        if slot >= self.show.total_slots() {
            return Err(SessionError::InvalidCue(slot));
        }
        let loaded = *self.loaded.get(&slot).ok_or(SessionError::NoOutput)?;
        let live = self.outputs[loaded.output]
            .live
            .as_mut()
            .ok_or(SessionError::NoOutput)?;
        Ok((live, loaded))
    }

    /// Plays a cue from the start (restarting it if playing). Firing an
    /// empty pad does nothing.
    /// Fires a pad, as pressing it does: plays it from the start, or if it
    /// is already playing, releases a vamp or applies its retrigger setting
    /// (restart, stop or fade). Starting a cue fades out others in its
    /// exclusive group and applies its ducking. Firing an empty pad does
    /// nothing.
    pub fn fire_cue(&mut self, slot: u32) -> Result<()> {
        if slot >= self.show.total_slots() {
            return Err(SessionError::InvalidCue(slot));
        }
        self.fire(slot)
    }

    /// Plays a cue from `seconds` into its timeline.
    pub fn play_cue_from(&mut self, slot: u32, seconds: f64) -> Result<()> {
        let (live, _) = self.live_for(slot)?;
        let player = PlayerId(slot);
        let position = seconds_to_samples(seconds, live.config.sample_rate);
        // Seek crossfades if it is already playing; play starts it if not.
        live.controller.seek(player, position)?;
        live.controller.play(player)?;
        Ok(())
    }

    /// Stops a cue, fading out over `fade_seconds` (0 = immediately).
    pub fn stop_cue(&mut self, slot: u32, fade_seconds: f64) -> Result<()> {
        self.recent_fires.remove(&slot);
        let (live, _) = self.live_for(slot)?;
        let fade = seconds_to_samples(fade_seconds, live.config.sample_rate);
        if fade == 0 {
            live.controller.stop(PlayerId(slot))?;
        } else {
            live.controller.fade_out_and_stop(PlayerId(slot), fade)?;
        }
        Ok(())
    }

    /// Stops everything on every output.
    pub fn stop_all(&mut self, fade_seconds: f64) -> Result<()> {
        self.recent_fires.clear();
        for live in self.outputs.iter_mut().filter_map(|o| o.live.as_mut()) {
            let fade = seconds_to_samples(fade_seconds, live.config.sample_rate);
            live.controller.stop_all(fade)?;
        }
        Ok(())
    }

    pub fn set_master_gain(&mut self, gain: f32) -> Result<()> {
        self.master_gain = gain;
        for live in self.outputs.iter_mut().filter_map(|o| o.live.as_mut()) {
            live.controller.set_master_gain(gain)?;
        }
        Ok(())
    }

    pub fn master_gain(&self) -> f32 {
        self.master_gain
    }

    pub fn cue_state(&self, slot: u32) -> Result<CueState> {
        if slot >= self.show.total_slots() {
            return Err(SessionError::InvalidCue(slot));
        }
        let doc = self.show.cue(slot);
        let mut state = CueState {
            assigned: doc.is_some(),
            looping: doc.is_some_and(|c| c.looping),
            ..CueState::default()
        };
        if let Some(loaded) = self.loaded.get(&slot)
            && let Some(live) = &self.outputs[loaded.output].live
        {
            let player = PlayerId(slot);
            state.ready = true;
            state.playing = live.controller.is_playing(player);
            state.holding = live.controller.is_holding(player);
            state.position_seconds =
                samples_to_seconds(live.controller.position(player), live.config.sample_rate);
            state.duration_seconds = loaded.length_seconds;
        }
        Ok(state)
    }

    /// Problems with cues (missing files, bad markers), for display.
    pub fn problems(&self) -> &[CueProblem] {
        &self.problems
    }

    /// One-off messages for the operator (failovers, reconnections).
    pub fn take_notices(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notices)
    }

    // ---- Housekeeping -------------------------------------------------

    /// Drains engine events, frees retired audio, and handles device
    /// failures. Call regularly from the UI (e.g. every 30 ms).
    pub fn poll(&mut self) {
        for live in self.outputs.iter_mut().filter_map(|o| o.live.as_mut()) {
            while live.controller.poll_event().is_some() {}
            live.controller.collect_garbage();
        }
        self.update_ducking();
        let now = Instant::now();
        for index in 0..self.outputs.len() {
            let failed = self.outputs[index]
                .live
                .as_ref()
                .map(|l| l.stream.status().health() != StreamHealth::Running);
            match failed {
                Some(true) => self.fail_over(index),
                Some(false) => {}
                None => {
                    if self.outputs[index].retry_at.is_some_and(|t| t <= now)
                        && self.open_output(index)
                    {
                        let state = &self.outputs_state()[index];
                        self.notices.push(format!(
                            "Output \"{}\" is back on \"{}\".",
                            state.name,
                            state.device_name.as_deref().unwrap_or("?")
                        ));
                    }
                }
            }
        }
    }

    /// The device behind output `index` failed: reopen on the next best
    /// device and resume whatever was playing, where it was.
    fn fail_over(&mut self, index: usize) {
        let Some(old) = self.outputs[index].live.take() else {
            return;
        };
        let lost = old.device.name.clone();
        let old_rate = old.config.sample_rate;
        let resume: Vec<(u32, f64)> = self
            .loaded
            .iter()
            .filter(|(_, l)| l.output == index)
            .filter(|(slot, _)| old.controller.is_playing(PlayerId(**slot)))
            .map(|(slot, _)| {
                let position = old.controller.position(PlayerId(*slot));
                (*slot, samples_to_seconds(position, old_rate))
            })
            .collect();
        drop(old);

        let name = self.show.outputs[index].name.clone();
        if self.open_output(index) {
            for (slot, seconds) in resume {
                let _ = self.play_cue_from(slot, seconds);
            }
            let now_on = self.outputs[index]
                .live
                .as_ref()
                .map_or_else(String::new, |l| l.device.name.clone());
            self.notices.push(format!(
                "Output \"{name}\" lost \"{lost}\"; now playing on \"{now_on}\"."
            ));
        } else {
            self.notices.push(format!(
                "Output \"{name}\" lost \"{lost}\" and no other device is available. Retrying…"
            ));
        }
    }
}
