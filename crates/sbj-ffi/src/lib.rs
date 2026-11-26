//! C ABI over the session, consumed by the Swift app (and by a future
//! Windows app). `include/sportsball_jukebox.h` must be kept in sync with
//! this file; `tests/smoke.c` compiles against the header in CI to catch
//! drift.
//!
//! Conventions:
//! * Every function is panic-safe: a panic is caught and reported as
//!   `SBJ_STATUS_INTERNAL_ERROR` rather than unwinding into Swift.
//! * Null pointers are tolerated and reported as `SBJ_STATUS_NULL_POINTER`.
//! * On failure, a human-readable message is available from
//!   `sbj_last_error_message` on the same thread.
//! * A session may be used from any thread; calls are serialised by an
//!   internal lock that the audio thread never touches. Slow work (decoding
//!   audio files) happens *outside* that lock, so firing cues is never
//!   blocked by a file load.
//! * Structured data (shows, cues, outputs, status) crosses as JSON, using
//!   the show file's schema (see `sbj-session/src/show.rs`). Strings
//!   returned by the library must be freed with `sbj_string_free`.

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use sbj_backend::system::SystemBackend;
use sbj_backend::{AudioBackend, DeviceInfo};
use sbj_control::Sink;
use sbj_control::midi::MidiListener;
use sbj_control::osc::OscListener;
use sbj_control::remote::{Handler, RemoteServer, Reply};
use sbj_engine::EngineError;
use sbj_session::{
    CheckLevel, CueDoc, DecodeReport, LearnTarget, OutputDoc, OutputStatus, Session, SessionError,
    Show, decode_files, waveform,
};
use serde_json::json;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SbjStatus {
    Ok = 0,
    NullPointer = 1,
    InvalidArgument = 2,
    QueueFull = 3,
    InternalError = 4,
    NoOutput = 5,
    DeviceError = 6,
    MediaError = 7,
    Truncated = 8,
    ShowError = 9,
}

pub const SBJ_ID_LEN: usize = 512;
pub const SBJ_NAME_LEN: usize = 256;

#[repr(C)]
pub struct SbjDeviceInfo {
    pub id: [c_char; SBJ_ID_LEN],
    pub name: [c_char; SBJ_NAME_LEN],
    pub output_channels: u16,
    pub sample_rate: u32,
    pub is_default: bool,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SbjCueState {
    pub assigned: bool,
    pub playing: bool,
    pub looping: bool,
    /// Loaded on an open output, i.e. can fire.
    pub ready: bool,
    /// Vamping: firing the pad again lets it continue past its vamp.
    pub holding: bool,
    /// Playlist pads: index of the song playing or last played in the
    /// playlist's items; `u32::MAX` if none / not a playlist.
    pub item: u32,
    /// Playlist pads: the song the next press or auto-advance plays;
    /// `u32::MAX` if none.
    pub next_item: u32,
    pub position_seconds: f64,
    pub duration_seconds: f64,
}

/// Opaque session handle.
pub struct SbjSession {
    // Declared first so listener threads stop before anything else drops.
    control: Mutex<Listeners>,
    shared: Arc<Shared>,
}

/// The parts MIDI/OSC listener threads also reach.
struct Shared {
    session: Mutex<Session<SystemBackend>>,
    /// Snapshot from the last `sbj_session_device_count`, so indices stay
    /// stable while the UI reads them.
    devices: Mutex<Vec<DeviceInfo>>,
}

#[derive(Default)]
struct Listeners {
    midi: Option<MidiListener>,
    osc: Option<OscListener>,
    remote: Option<RemoteServer>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Session<SystemBackend>> {
        // A panic while locked was already reported; the session's state
        // is still consistent (every mutation is a single assignment or an
        // engine command), so keep going rather than failing forever.
        self.session.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl SbjSession {
    fn lock(&self) -> MutexGuard<'_, Session<SystemBackend>> {
        self.shared.lock()
    }

    fn devices(&self) -> MutexGuard<'_, Vec<DeviceInfo>> {
        self.shared
            .devices
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn listeners(&self) -> MutexGuard<'_, Listeners> {
        self.control.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Delivers control messages straight to the session from listener
    /// threads. Holds only a weak reference so it never keeps a destroyed
    /// session alive.
    fn sink(&self) -> Sink {
        let shared = Arc::downgrade(&self.shared);
        Arc::new(move |message| {
            if let Some(shared) = shared.upgrade() {
                // Never let a panic escape into CoreMIDI's thread.
                let _ = catch_unwind(AssertUnwindSafe(|| shared.lock().handle_control(message)));
            }
        })
    }

    /// Answers the phone/tablet remote's API requests. Weak, like `sink`.
    fn remote_handler(&self) -> Handler {
        let shared = Arc::downgrade(&self.shared);
        Arc::new(move |method, path| {
            let Some(shared) = shared.upgrade() else {
                return Reply::error(503, "the app is closing");
            };
            catch_unwind(AssertUnwindSafe(|| {
                match shared.lock().handle_remote(method, path) {
                    Ok(state) => Reply::json(state.to_string()),
                    Err(e) => Reply::error(e.status, &e.message),
                }
            }))
            .unwrap_or_else(|_| Reply::error(500, "internal error"))
        })
    }
}

// ---------------------------------------------------------------------
// Error reporting and helpers
// ---------------------------------------------------------------------

thread_local! {
    static LAST_ERROR: RefCell<String> = const { RefCell::new(String::new()) };
}

fn set_error(message: impl Into<String>) {
    LAST_ERROR.with(|e| *e.borrow_mut() = message.into());
}

fn fail(status: SbjStatus, message: impl Into<String>) -> SbjStatus {
    set_error(message);
    status
}

fn session_status(r: Result<(), SessionError>) -> SbjStatus {
    match r {
        Ok(()) => SbjStatus::Ok,
        Err(e) => {
            let status = match &e {
                SessionError::InvalidCue(_) | SessionError::InvalidOutput(_) => {
                    SbjStatus::InvalidArgument
                }
                SessionError::NoOutput => SbjStatus::NoOutput,
                SessionError::Backend(_) => SbjStatus::DeviceError,
                SessionError::Media(_) => SbjStatus::MediaError,
                SessionError::Show(_) => SbjStatus::ShowError,
                SessionError::Engine(EngineError::QueueFull) => SbjStatus::QueueFull,
                SessionError::Engine(_) => SbjStatus::InternalError,
            };
            fail(status, e.to_string())
        }
    }
}

fn guard<F: FnOnce() -> SbjStatus>(f: F) -> SbjStatus {
    catch_unwind(AssertUnwindSafe(f))
        .unwrap_or_else(|_| fail(SbjStatus::InternalError, "internal error (panic)"))
}

/// Copies `s` into a fixed C buffer, NUL-terminated. Returns false if it
/// had to be truncated (at a UTF-8 character boundary).
fn copy_str(s: &str, dst: &mut [c_char]) -> bool {
    let max = dst.len().saturating_sub(1);
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    for (d, b) in dst.iter_mut().zip(&s.as_bytes()[..end]) {
        *d = *b as c_char;
    }
    if let Some(nul) = dst.get_mut(end) {
        *nul = 0;
    }
    end == s.len()
}

/// # Safety
/// `ptr` must be null or a valid NUL-terminated string.
unsafe fn read_str<'a>(ptr: *const c_char) -> Result<Option<&'a str>, SbjStatus> {
    if ptr.is_null() {
        return Ok(None);
    }
    // SAFETY: guaranteed by the caller.
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map(Some)
        .map_err(|_| fail(SbjStatus::InvalidArgument, "string is not valid UTF-8"))
}

/// # Safety
/// As [`read_str`]; null is an error.
unsafe fn required_str<'a>(ptr: *const c_char, what: &str) -> Result<&'a str, SbjStatus> {
    // SAFETY: forwarded caller contract.
    match unsafe { read_str(ptr) }? {
        Some(s) => Ok(s),
        None => Err(fail(SbjStatus::NullPointer, format!("{what} is null"))),
    }
}

/// Hands a string to C; free with `sbj_string_free`.
fn to_c(s: String) -> *mut c_char {
    CString::new(s.replace('\0', ""))
        .map(CString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

/// Frees a string returned by this library. Null is ignored.
///
/// # Safety
/// `s` must be null or a string returned by this library, not yet freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_string_free(s: *mut c_char) {
    if !s.is_null() {
        // SAFETY: the pointer came from `CString::into_raw` in `to_c`.
        drop(unsafe { CString::from_raw(s) });
    }
}

/// Copies the calling thread's last error message into `buf` (always
/// NUL-terminated if `len > 0`). Returns the message length in bytes,
/// excluding the terminator, so callers can detect truncation.
///
/// # Safety
/// `buf` must be null or valid for writes of `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_last_error_message(buf: *mut c_char, len: usize) -> usize {
    LAST_ERROR.with(|e| {
        let e = e.borrow();
        if !buf.is_null() && len > 0 {
            // SAFETY: guaranteed by the caller.
            let dst = unsafe { std::slice::from_raw_parts_mut(buf, len) };
            copy_str(&e, dst);
        }
        e.len()
    })
}

/// Library version as a static NUL-terminated string.
#[unsafe(no_mangle)]
pub extern "C" fn sbj_version() -> *const c_char {
    static VERSION: &CStr =
        match CStr::from_bytes_with_nul(concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes()) {
            Ok(v) => v,
            Err(_) => c"unknown",
        };
    VERSION.as_ptr()
}

/// Returns true if the caller's view of the shared structs matches this
/// library's. Callers pass `sizeof` each struct; the Swift app checks this
/// at startup so a stale header fails loudly instead of corrupting memory.
#[unsafe(no_mangle)]
pub extern "C" fn sbj_abi_check(device_info_size: usize, cue_state_size: usize) -> bool {
    device_info_size == std::mem::size_of::<SbjDeviceInfo>()
        && cue_state_size == std::mem::size_of::<SbjCueState>()
}

// ---------------------------------------------------------------------
// Session lifecycle
// ---------------------------------------------------------------------

/// Creates a session with an empty show. No audio device is opened until
/// `sbj_session_open_outputs`.
#[unsafe(no_mangle)]
pub extern "C" fn sbj_session_create() -> *mut SbjSession {
    catch_unwind(|| {
        Box::into_raw(Box::new(SbjSession {
            control: Mutex::new(Listeners::default()),
            shared: Arc::new(Shared {
                session: Mutex::new(Session::new(SystemBackend::new())),
                devices: Mutex::new(Vec::new()),
            }),
        }))
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Closes all outputs and frees the session. Null is ignored.
///
/// # Safety
/// `session` must be null or from `sbj_session_create`, not already
/// destroyed, and not in use on another thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_destroy(session: *mut SbjSession) {
    if !session.is_null() {
        // SAFETY: the pointer came from `Box::into_raw`.
        drop(unsafe { Box::from_raw(session) });
    }
}

/// # Safety
/// `session` must be null or a live session.
unsafe fn with_session(
    session: *const SbjSession,
    f: impl FnOnce(&SbjSession) -> SbjStatus,
) -> SbjStatus {
    // SAFETY: guaranteed by the caller.
    match unsafe { session.as_ref() } {
        Some(s) => guard(|| f(s)),
        None => fail(SbjStatus::NullPointer, "session is null"),
    }
}

/// Runs `f` and returns its string (or null on failure) for C.
///
/// # Safety
/// `session` must be null or a live session.
unsafe fn session_string(
    session: *const SbjSession,
    f: impl FnOnce(&SbjSession) -> Result<String, SbjStatus>,
) -> *mut c_char {
    let mut out = std::ptr::null_mut();
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| match f(s) {
            Ok(text) => {
                out = to_c(text);
                SbjStatus::Ok
            }
            Err(status) => status,
        })
    };
    out
}

/// Opens every output's audio device and loads cues onto them.
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_open_outputs(session: *const SbjSession) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            s.lock().open_outputs();
            SbjStatus::Ok
        })
    }
}

/// Housekeeping and device failover; call every ~30 ms from the UI.
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_poll(session: *const SbjSession) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            s.lock().poll();
            SbjStatus::Ok
        })
    }
}

/// Name of the audio system in use ("CoreAudio" on macOS).
///
/// # Safety
/// `session` must be null or a live session; `buf` valid for `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_backend_name(
    session: *const SbjSession,
    buf: *mut c_char,
    len: usize,
) -> SbjStatus {
    if buf.is_null() || len == 0 {
        return fail(SbjStatus::NullPointer, "buffer is null");
    }
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            let name = s.lock().backend().name();
            // SAFETY: caller guarantees `buf` holds `len` bytes.
            let dst = std::slice::from_raw_parts_mut(buf, len);
            if copy_str(name, dst) {
                SbjStatus::Ok
            } else {
                SbjStatus::Truncated
            }
        })
    }
}

// ---------------------------------------------------------------------
// Devices
// ---------------------------------------------------------------------

/// Re-scans output devices and returns how many there are. Read each with
/// `sbj_session_device_info`.
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_device_count(session: *const SbjSession) -> u32 {
    let mut count = 0;
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            // Enumerate without holding the session lock (it can be slow).
            let devices = match SystemBackend::new().output_devices() {
                Ok(d) => d,
                Err(e) => return session_status(Err(e.into())),
            };
            count = devices.len() as u32;
            *s.devices() = devices;
            SbjStatus::Ok
        })
    };
    count
}

/// # Safety
/// `session` must be null or a live session; `out` null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_device_info(
    session: *const SbjSession,
    index: u32,
    out: *mut SbjDeviceInfo,
) -> SbjStatus {
    // SAFETY: guaranteed by the caller.
    let Some(out) = (unsafe { out.as_mut() }) else {
        return fail(SbjStatus::NullPointer, "out is null");
    };
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            let devices = s.devices();
            let Some(d) = devices.get(index as usize) else {
                return fail(SbjStatus::InvalidArgument, "device index out of range");
            };
            let id_fits = copy_str(&d.id.0, &mut out.id);
            copy_str(&d.name, &mut out.name);
            out.output_channels = d.output_channels;
            out.sample_rate = d.sample_rate;
            out.is_default = d.is_default;
            if id_fits {
                SbjStatus::Ok
            } else {
                fail(SbjStatus::Truncated, "device id too long")
            }
        })
    }
}

// ---------------------------------------------------------------------
// Show documents
// ---------------------------------------------------------------------

/// The current show as JSON (the show file format). Null on failure.
/// Free with `sbj_string_free`.
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_show_json(session: *const SbjSession) -> *mut c_char {
    // SAFETY: forwarded caller contract.
    unsafe { session_string(session, |s| Ok(s.lock().show().to_json())) }
}

/// Status as JSON, for the UI (poll a few times a second):
/// `{"dirty": bool, "path": string|null, "outputs": [...],
///   "problems": [{"slot", "message"}], "notices": [string]}`.
/// Notices are returned once. Free with `sbj_string_free`.
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_status_json(session: *const SbjSession) -> *mut c_char {
    // SAFETY: forwarded caller contract.
    unsafe {
        session_string(session, |s| {
            // Lock order: listeners, then session (never the reverse; MIDI
            // callbacks hold the session while listeners are torn down).
            let (midi_inputs, osc_port, remote_port) = {
                let listeners = s.listeners();
                (
                    listeners.midi.as_ref().map(|m| m.inputs().to_vec()),
                    listeners.osc.as_ref().map(OscListener::port),
                    listeners.remote.as_ref().map(RemoteServer::port),
                )
            };
            let mut session = s.lock();
            let outputs: Vec<_> = session
                .outputs_state()
                .into_iter()
                .map(|o| {
                    json!({
                        "name": o.name,
                        "status": match o.status {
                            OutputStatus::Running => "running",
                            OutputStatus::OnBackup => "on_backup",
                            OutputStatus::Offline => "offline",
                        },
                        "device_id": o.device_id,
                        "device_name": o.device_name,
                        "sample_rate": o.sample_rate,
                        "channels": o.channels,
                        "xruns": o.xruns,
                        "error": o.error,
                    })
                })
                .collect();
            let problems: Vec<_> = session
                .problems()
                .iter()
                .map(|p| json!({"slot": p.slot, "message": p.message}))
                .collect();
            let preview = session.preview_state().map(|p| {
                json!({
                    "slot": p.slot,
                    "file": p.file,
                    "playing": p.playing,
                    "position": p.position_seconds,
                    "duration": p.duration_seconds,
                })
            });
            let mut notices = session.take_notices();
            if let Some(error) = session.take_log_error() {
                notices.push(error);
            }
            let status = json!({
                "game": session.game(),
                "preview_output": session.show().preview_output,
                "preview": preview,
                "bank": session.current_bank(),
                "standby": session.standby(),
                "played": session.played(),
                "watch_imports": session.watch_imports(),
                "missing_watch_folders": session.missing_watch_folders(),
                "undo": session.undo_label(),
                "redo": session.redo_label(),
                "last": session.last_playing(),
                "single_play": session.show().single_play,
                "learning": session.learning().map(target_name),
                "last_control": session.last_control(),
                "midi_inputs": midi_inputs,
                "osc_port": osc_port,
                "remote_port": remote_port,
                "dirty": session.is_dirty(),
                "path": session.show_path().map(|p| p.display().to_string()),
                "outputs": outputs,
                "problems": problems,
                "notices": notices,
            });
            Ok(status.to_string())
        })
    }
}

/// Replaces the show with a new empty one and opens its outputs.
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_new_show(session: *const SbjSession) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            s.lock().new_show();
            SbjStatus::Ok
        })
    }
}

/// Opens a show file: reads it, decodes all its audio (slow; done without
/// blocking playback controls), then switches to it. On failure the
/// current show is untouched. Missing audio is reported in the status's
/// `problems`, not as a failure.
///
/// # Safety
/// `session` must be null or a live session; `path` a valid string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_open_show(
    session: *const SbjSession,
    path: *const c_char,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    let path = match unsafe { required_str(path, "path") } {
        Ok(p) => PathBuf::from(p),
        Err(status) => return status,
    };
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            let show = match Show::load(&path) {
                Ok(show) => show,
                Err(e) => return fail(SbjStatus::ShowError, e.to_string()),
            };
            let rates = s.lock().output_rates();
            let decoded = decode_files(&show.files(), path.parent(), &rates);
            s.lock().install_show(show, Some(path), decoded);
            SbjStatus::Ok
        })
    }
}

/// Saves the show to `path`, or to its current file if `path` is null.
///
/// # Safety
/// `session` must be null or a live session; `path` null or a valid
/// string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_save_show(
    session: *const SbjSession,
    path: *const c_char,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    let path = match unsafe { read_str(path) } {
        Ok(p) => p.map(PathBuf::from),
        Err(status) => return status,
    };
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            let mut session = s.lock();
            let Some(path) = path.or_else(|| session.show_path().map(Path::to_path_buf)) else {
                return fail(SbjStatus::InvalidArgument, "the show has no file yet");
            };
            session_status(session.save_show(&path))
        })
    }
}

/// SLOW: saves the show to `path` with copies of every file it uses
/// (pads, playlists, the library, pad pictures) in `Audio` and `Images`
/// folders beside it, and points the show at those copies, so that folder
/// can be moved anywhere. Copying happens without holding the session, so
/// pads keep firing. Files that can't be found are skipped and reported as
/// a notice.
///
/// # Safety
/// `session` must be null or a live session; `path` a valid string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_save_with_audio(
    session: *const SbjSession,
    path: *const c_char,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    let path = match unsafe { required_str(path, "path") } {
        Ok(p) => PathBuf::from(p),
        Err(status) => return status,
    };
    let Some(folder) = path.parent().map(Path::to_path_buf) else {
        return fail(SbjStatus::InvalidArgument, "the show path has no folder");
    };
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            if let Err(e) = std::fs::create_dir_all(&folder) {
                return fail(
                    SbjStatus::ShowError,
                    format!("could not create the folder: {e}"),
                );
            }
            let plan = s.lock().collect_plan(&folder);
            if let Err(e) = plan.copy() {
                return fail(
                    SbjStatus::ShowError,
                    format!("could not copy the audio: {e}"),
                );
            }
            let mut session = s.lock();
            session.apply_collect(&plan);
            session_status(session.save_show(&path))
        })
    }
}

/// Undoes the last recorded show edit (see `sbj_session_record_undo`).
/// Nothing to undo is not an error.
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_undo(session: *const SbjSession) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            s.lock().undo();
            SbjStatus::Ok
        })
    }
}

/// Redoes the last undone edit.
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_redo(session: *const SbjSession) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            s.lock().redo();
            SbjStatus::Ok
        })
    }
}

/// Marks the show's changes since the last mark as one undoable step
/// named `label` (e.g. "Paste Style"). Returns true if anything changed
/// (and so a step was recorded).
///
/// # Safety
/// `session` must be null or a live session; `label` null or a valid
/// string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_record_undo(
    session: *const SbjSession,
    label: *const c_char,
) -> bool {
    // SAFETY: forwarded caller contract.
    let label = match unsafe { read_str(label) } {
        Ok(l) => l.unwrap_or("Change").to_owned(),
        Err(_) => return false,
    };
    // SAFETY: forwarded caller contract.
    let Some(s) = (unsafe { session.as_ref() }) else {
        set_error("session is null");
        return false;
    };
    catch_unwind(AssertUnwindSafe(|| s.lock().record_undo(&label))).unwrap_or(false)
}

/// Sets the show's name and grid from JSON:
/// `{"name", "slots" (pads per bank), "columns", "banks": [names]}`.
///
/// # Safety
/// `session` must be null or a live session; `json` a valid string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_set_show_settings_json(
    session: *const SbjSession,
    json: *const c_char,
) -> SbjStatus {
    #[derive(serde::Deserialize)]
    struct Settings {
        name: String,
        slots: u32,
        columns: u32,
        banks: Vec<String>,
        #[serde(default)]
        stop_all_fade: Option<f64>,
        /// Absent: unchanged; null: off; a number: target LUFS.
        #[serde(default)]
        level_match: Option<serde_json::Value>,
        /// Absent: unchanged; null: off; a string: colour of played pads.
        #[serde(default)]
        played_color: Option<serde_json::Value>,
        #[serde(default)]
        mark_played: Option<bool>,
    }
    // SAFETY: forwarded caller contract.
    let settings: Settings = match unsafe { required_str(json, "json") }.and_then(|j| {
        serde_json::from_str(j)
            .map_err(|e| fail(SbjStatus::InvalidArgument, format!("bad settings: {e}")))
    }) {
        Ok(s) => s,
        Err(status) => return status,
    };
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            let mut session = s.lock();
            if let Some(fade) = settings.stop_all_fade {
                session.set_stop_all_fade(fade);
            }
            if let Some(level) = &settings.level_match {
                session.set_level_match(level.as_f64());
            }
            if settings.played_color.is_some() || settings.mark_played.is_some() {
                let color = match &settings.played_color {
                    Some(value) => value.as_str().map(str::to_owned),
                    None => session.show().played_color.clone(),
                };
                let mark = settings.mark_played.unwrap_or(session.show().mark_played);
                session.set_played_style(color, mark);
            }
            session_status(session.set_show_settings(
                &settings.name,
                settings.slots,
                settings.columns,
                settings.banks,
            ))
        })
    }
}

/// What was measured about a loaded file, as JSON (null if not loaded):
/// `{"duration", "loudness": LUFS|null, "peak_db": dBFS|null,
/// "lead_silence", "tail_silence", "level_gain_db": dB|null}`.
/// `level_gain_db` is what the show's level matching would apply (null
/// when it is off or the file is silent). Free with `sbj_string_free`.
///
/// # Safety
/// `session` must be null or a live session; `file` a valid string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_sound_info_json(
    session: *const SbjSession,
    file: *const c_char,
) -> *mut c_char {
    // SAFETY: forwarded caller contract.
    let file = match unsafe { required_str(file, "file") } {
        Ok(f) => f.to_owned(),
        Err(_) => return std::ptr::null_mut(),
    };
    // SAFETY: forwarded caller contract.
    unsafe {
        session_string(session, |s| {
            let session = s.lock();
            let Some(sound) = session.sound(&file) else {
                return Ok("null".into());
            };
            let a = sound.analysis();
            let finite = |x: f64| x.is_finite().then_some(x);
            let gain = session
                .show()
                .level_match
                .filter(|_| a.loudness_lufs.is_some())
                .map(|target| {
                    let g = a.gain_to(
                        target,
                        sbj_session::show::LEVEL_MATCH_MAX_BOOST_DB,
                        sbj_session::show::LEVEL_MATCH_CEILING_DBFS,
                    );
                    20.0 * f64::from(g).log10()
                });
            Ok(json!({
                "duration": sound.duration_seconds(),
                "loudness": a.loudness_lufs,
                "peak_db": finite(a.peak_dbfs()),
                "lead_silence": a.lead_silence,
                "tail_silence": a.tail_silence,
                "level_gain_db": gain,
            })
            .to_string())
        })
    }
}

/// Replaces the game rundown with a JSON array of steps
/// `[{"id", "pad"?: slot, "text"?: string}]` (a step without a pad is a
/// note).
///
/// # Safety
/// `session` must be null or a live session; `json` a valid string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_set_rundown_json(
    session: *const SbjSession,
    json: *const c_char,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    let items: Vec<sbj_session::RundownItem> =
        match unsafe { required_str(json, "json") }.and_then(|j| {
            serde_json::from_str(j)
                .map_err(|e| fail(SbjStatus::InvalidArgument, format!("bad rundown: {e}")))
        }) {
            Ok(items) => items,
            Err(status) => return status,
        };
    // SAFETY: forwarded caller contract.
    unsafe { with_session(session, |s| session_status(s.lock().set_rundown(items))) }
}

/// Fires the rundown's standby step (as pressing its pad does) and moves
/// standby on. Does nothing when the rundown is finished.
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_rundown_go(session: *const SbjSession) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            session_status(s.lock().rundown_go().map(|_| ()))
        })
    }
}

/// Puts rundown step `index` on standby (a note moves on to the next pad
/// step); a negative index clears standby (finished).
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_set_standby(
    session: *const SbjSession,
    index: i32,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            s.lock().set_standby(usize::try_from(index).ok());
            SbjStatus::Ok
        })
    }
}

/// Moves standby to the next (`forward`) or previous pad step without
/// firing.
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_step_standby(
    session: *const SbjSession,
    forward: bool,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            s.lock().step_standby(forward);
            SbjStatus::Ok
        })
    }
}

/// Marks (`played`) or unmarks a pad as played (status JSON "played").
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_set_played(
    session: *const SbjSession,
    slot: u32,
    played: bool,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            s.lock().set_played(slot, played);
            SbjStatus::Ok
        })
    }
}

/// Unmarks every pad in `bank` (0-based) as played; negative = all banks.
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_reset_played(
    session: *const SbjSession,
    bank: i32,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            s.lock().reset_played(u32::try_from(bank).ok());
            SbjStatus::Ok
        })
    }
}

/// The event log as JSON, oldest first: `[{"time": unix seconds,
/// "level": "info"|"warning"|"error", "kind": string, "message": string}]`
/// (glitches with what was playing, device changes, failed fires,
/// warnings). Null on failure. Free with `sbj_string_free`.
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_events_json(session: *const SbjSession) -> *mut c_char {
    // SAFETY: forwarded caller contract.
    unsafe {
        session_string(session, |s| {
            let session = s.lock();
            Ok(serde_json::to_string(session.events()).unwrap_or_else(|_| "[]".into()))
        })
    }
}

/// Starts every output's glitch count again from zero (the event log
/// keeps the history).
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_reset_glitches(session: *const SbjSession) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            s.lock().reset_glitches();
            SbjStatus::Ok
        })
    }
}

/// Moves the preview's playhead to `seconds`, playing it again if it had
/// stopped (the preview player's scrub bar).
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_seek_preview(
    session: *const SbjSession,
    seconds: f64,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    unsafe { with_session(session, |s| session_status(s.lock().seek_preview(seconds))) }
}

/// Fades out (`fade`, over the pad's own fade-out) or stops the most
/// recently fired pad that is still playing. Nothing playing is not an
/// error.
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_stop_last(
    session: *const SbjSession,
    fade: bool,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            session_status(s.lock().stop_last(fade).map(|_| ()))
        })
    }
}

/// Multiplay on (default) lets a bank's pads play over each other; off
/// (single-play) makes firing a pad fade out the bank's other pads.
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_set_bank_multiplay(
    session: *const SbjSession,
    bank: u32,
    multiplay: bool,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            s.lock().set_bank_multiplay(bank, multiplay);
            SbjStatus::Ok
        })
    }
}

/// Sets the settings given to new pads from JSON (see the show file's
/// `pad_defaults`); null or "null" restores the built-in defaults.
///
/// # Safety
/// `session` must be null or a live session; `json` null or a valid
/// string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_set_pad_defaults_json(
    session: *const SbjSession,
    json: *const c_char,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    let defaults: Option<sbj_session::PadDefaults> = match unsafe { read_str(json) } {
        Err(status) => return status,
        Ok(None) => None,
        Ok(Some(j)) => match serde_json::from_str(j) {
            Ok(d) => d,
            Err(e) => return fail(SbjStatus::InvalidArgument, format!("bad pad defaults: {e}")),
        },
    };
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            s.lock().set_pad_defaults(defaults);
            SbjStatus::Ok
        })
    }
}

/// Replaces the show's colour presets with a JSON array
/// `[{"name", "value": "#RRGGBB"}]`. With `recolor`, pads using a preset
/// whose colour changed (matched by name) take the new colour.
///
/// # Safety
/// `session` must be null or a live session; `json` a valid string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_set_show_colors_json(
    session: *const SbjSession,
    json: *const c_char,
    recolor: bool,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    let colors: Vec<sbj_session::ShowColor> =
        match unsafe { required_str(json, "json") }.and_then(|j| {
            serde_json::from_str(j)
                .map_err(|e| fail(SbjStatus::InvalidArgument, format!("bad colours: {e}")))
        }) {
            Ok(c) => c,
            Err(status) => return status,
        };
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            s.lock().set_show_colors(colors, recolor);
            SbjStatus::Ok
        })
    }
}

/// The show's audio library as JSON: `[{"file", "used_by": [slot],
/// "loaded", "duration": s|null, "loudness": LUFS|null, "error":
/// string|null}]`. Free with `sbj_string_free`.
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_library_json(session: *const SbjSession) -> *mut c_char {
    // SAFETY: forwarded caller contract.
    unsafe {
        session_string(session, |s| {
            let entries: Vec<_> = s
                .lock()
                .library_entries()
                .into_iter()
                .map(|e| {
                    json!({
                        "file": e.file,
                        "used_by": e.used_by,
                        "loaded": e.loaded,
                        "duration": e.duration_seconds,
                        "loudness": e.loudness_lufs,
                        "error": e.error,
                    })
                })
                .collect();
            Ok(serde_json::Value::from(entries).to_string())
        })
    }
}

/// Adds files (a JSON array of paths) to the library without decoding
/// them; files already there are skipped.
///
/// # Safety
/// `session` must be null or a live session; `json` a valid string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_add_to_library_json(
    session: *const SbjSession,
    json: *const c_char,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    let files: Vec<String> = match unsafe { required_str(json, "json") }.and_then(|j| {
        serde_json::from_str(j)
            .map_err(|e| fail(SbjStatus::InvalidArgument, format!("bad file list: {e}")))
    }) {
        Ok(files) => files,
        Err(status) => return status,
    };
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            s.lock().add_to_library(files);
            SbjStatus::Ok
        })
    }
}

/// Sets the show's watch folders, whose new audio files are added to the
/// library: a JSON array `[{"path": string, "subfolders"?: bool}]`
/// (`subfolders` defaults to true). Status JSON `"watch_imports"` counts
/// imports (reload the library when it changes) and
/// `"missing_watch_folders"` lists folders that can't be read.
///
/// # Safety
/// `session` must be null or a live session; `json` a valid string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_set_watch_folders_json(
    session: *const SbjSession,
    json: *const c_char,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    let folders: Vec<sbj_session::WatchFolder> = match unsafe { required_str(json, "json") }
        .and_then(|j| {
            serde_json::from_str(j).map_err(|e| {
                fail(
                    SbjStatus::InvalidArgument,
                    format!("bad watch folder list: {e}"),
                )
            })
        }) {
        Ok(folders) => folders,
        Err(status) => return status,
    };
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            s.lock().set_watch_folders(folders);
            SbjStatus::Ok
        })
    }
}

/// Removes a file from the library; refused while a pad uses it.
///
/// # Safety
/// `session` must be null or a live session; `file` a valid string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_remove_from_library(
    session: *const SbjSession,
    file: *const c_char,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    let file = match unsafe { required_str(file, "file") } {
        Ok(f) => f.to_owned(),
        Err(status) => return status,
    };
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            session_status(s.lock().remove_from_library(&file))
        })
    }
}

/// SLOW the first time (decodes the file, outside the session lock):
/// plays a library file on the preview output; again stops it.
///
/// # Safety
/// `session` must be null or a live session; `file` a valid string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_preview_file(
    session: *const SbjSession,
    file: *const c_char,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    let file = match unsafe { required_str(file, "file") } {
        Ok(f) => f.to_owned(),
        Err(status) => return status,
    };
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            let report = preload(s, vec![file.clone()]);
            if let Some((_, why)) = report.failures.first() {
                return fail(SbjStatus::MediaError, format!("{file}: {why}"));
            }
            session_status(s.lock().preview_file(&file))
        })
    }
}

/// Where a file the show refers to (audio, or a pad picture) is on disk,
/// looking next to the show and in its `Audio` and `Images` folders if the
/// stored path is gone; NULL (no error) if it cannot be found. Free with
/// `sbj_string_free`.
///
/// # Safety
/// `session` must be null or a live session; `file` a valid string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_resolve_file(
    session: *const SbjSession,
    file: *const c_char,
) -> *mut c_char {
    // SAFETY: forwarded caller contract.
    let Ok(file) = (unsafe { required_str(file, "file") }).map(str::to_owned) else {
        return std::ptr::null_mut();
    };
    // SAFETY: forwarded caller contract.
    let Some(s) = (unsafe { session.as_ref() }) else {
        set_error("session is null");
        return std::ptr::null_mut();
    };
    catch_unwind(AssertUnwindSafe(|| {
        let dir = s
            .lock()
            .show_path()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        sbj_session::show::resolve_file(&file, dir.as_deref())
            .map_or(std::ptr::null_mut(), |p| to_c(p.display().to_string()))
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// Shows bank `bank` (0-based): hotkeys and OSC pad numbers refer to it.
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_set_bank(session: *const SbjSession, bank: u32) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    unsafe { with_session(session, |s| session_status(s.lock().set_bank(bank))) }
}

// ---------------------------------------------------------------------
// External control: MIDI, OSC, learn
// ---------------------------------------------------------------------

/// Turns MIDI input on (listening to every connected MIDI device) or off.
/// Calling it again with `true` rescans for devices.
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_enable_midi(
    session: *const SbjSession,
    enabled: bool,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            // Stop the old listener without holding any lock: it waits for
            // in-flight callbacks, which take the session lock.
            let old = s.listeners().midi.take();
            #[allow(clippy::drop_non_drop)] // only the stub (Linux) has no Drop
            drop(old);
            if !enabled {
                return SbjStatus::Ok;
            }
            match MidiListener::start(s.sink()) {
                Ok(midi) => {
                    s.listeners().midi = Some(midi);
                    SbjStatus::Ok
                }
                Err(e) => fail(SbjStatus::DeviceError, format!("MIDI unavailable: {e}")),
            }
        })
    }
}

/// Listens for OSC on UDP `port`, or stops listening if `port` is 0.
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_set_osc_port(
    session: *const SbjSession,
    port: u16,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            // As for MIDI: stop the old listener outside any lock.
            let old = s.listeners().osc.take();
            drop(old);
            if port == 0 {
                return SbjStatus::Ok;
            }
            match OscListener::start(port, s.sink()) {
                Ok(osc) => {
                    s.listeners().osc = Some(osc);
                    SbjStatus::Ok
                }
                Err(e) => fail(
                    SbjStatus::DeviceError,
                    format!("cannot listen on port {port}: {e}"),
                ),
            }
        })
    }
}

/// Serves the phone/tablet remote page on TCP `port` (all interfaces),
/// guarded by `pin` (4–12 digits), or stops it if `port` is 0 (`pin` may
/// then be null).
///
/// # Safety
/// `session` must be null or a live session; `pin` null or a valid string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_set_remote(
    session: *const SbjSession,
    port: u16,
    pin: *const c_char,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    let pin = match unsafe { read_str(pin) } {
        Ok(p) => p.map(str::to_owned),
        Err(status) => return status,
    };
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            // Stop the old server outside any lock (it joins its thread,
            // which may be waiting for the session lock).
            let old = s.listeners().remote.take();
            drop(old);
            if port == 0 {
                return SbjStatus::Ok;
            }
            let Some(pin) = pin else {
                return fail(SbjStatus::NullPointer, "pin is null");
            };
            if !(4..=12).contains(&pin.len()) || !pin.bytes().all(|b| b.is_ascii_digit()) {
                return fail(SbjStatus::InvalidArgument, "the PIN must be 4 to 12 digits");
            }
            match RemoteServer::start(port, pin, s.remote_handler()) {
                Ok(server) => {
                    s.listeners().remote = Some(server);
                    SbjStatus::Ok
                }
                Err(e) => fail(
                    SbjStatus::DeviceError,
                    format!("cannot serve the remote on port {port}: {e}"),
                ),
            }
        })
    }
}

fn parse_target(target: &str) -> Option<LearnTarget> {
    match target {
        "stop_all" => Some(LearnTarget::StopAll),
        "next_bank" => Some(LearnTarget::NextBank),
        "prev_bank" => Some(LearnTarget::PrevBank),
        "rundown_go" => Some(LearnTarget::RundownGo),
        "fade_last" => Some(LearnTarget::FadeLast),
        other => other
            .strip_prefix("cue:")
            .and_then(|n| n.parse().ok())
            .map(LearnTarget::Cue),
    }
}

fn target_name(target: LearnTarget) -> String {
    match target {
        LearnTarget::StopAll => "stop_all".into(),
        LearnTarget::NextBank => "next_bank".into(),
        LearnTarget::PrevBank => "prev_bank".into(),
        LearnTarget::RundownGo => "rundown_go".into(),
        LearnTarget::FadeLast => "fade_last".into(),
        LearnTarget::Cue(slot) => format!("cue:{slot}"),
    }
}

/// Starts MIDI learn: the next MIDI message is assigned to `target`
/// ("cue:<slot>", "stop_all", "next_bank" or "prev_bank"). Null cancels.
///
/// # Safety
/// `session` must be null or a live session; `target` null or a valid
/// string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_learn_midi(
    session: *const SbjSession,
    target: *const c_char,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    let target = match unsafe { read_str(target) } {
        Ok(None) => None,
        Ok(Some(t)) => match parse_target(t) {
            Some(t) => Some(t),
            None => return fail(SbjStatus::InvalidArgument, format!("unknown target {t}")),
        },
        Err(status) => return status,
    };
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            let mut session = s.lock();
            match target {
                Some(t) => session_status(session.start_learning(t)),
                None => {
                    session.cancel_learning();
                    SbjStatus::Ok
                }
            }
        })
    }
}

/// Removes the MIDI trigger from `target` (same names as learn).
///
/// # Safety
/// `session` must be null or a live session; `target` a valid string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_clear_midi(
    session: *const SbjSession,
    target: *const c_char,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    let target = match unsafe { required_str(target, "target") } {
        Ok(t) => match parse_target(t) {
            Some(t) => t,
            None => return fail(SbjStatus::InvalidArgument, format!("unknown target {t}")),
        },
        Err(status) => return status,
    };
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            s.lock().set_control_trigger(target, None);
            SbjStatus::Ok
        })
    }
}

/// The pre-game check as JSON:
/// `{"ready": bool, "memory_mb": number,
///   "checks": [{"level": "ok|info|warning|error", "message"}]}`
/// (most severe first). Free with `sbj_string_free`.
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_preflight_json(session: *const SbjSession) -> *mut c_char {
    // SAFETY: forwarded caller contract.
    unsafe {
        session_string(session, |s| {
            let report = s.lock().preflight();
            let checks: Vec<_> = report
                .checks
                .iter()
                .map(|c| {
                    json!({
                        "level": match c.level {
                            CheckLevel::Ok => "ok",
                            CheckLevel::Info => "info",
                            CheckLevel::Warning => "warning",
                            CheckLevel::Error => "error",
                        },
                        "message": c.message,
                    })
                })
                .collect();
            Ok(json!({
                "ready": report.ready(),
                "memory_mb": report.memory_bytes as f64 / (1024.0 * 1024.0),
                "checks": checks,
            })
            .to_string())
        })
    }
}

/// Replaces the output list from a JSON array of outputs (show format).
///
/// # Safety
/// `session` must be null or a live session; `json` a valid string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_set_outputs_json(
    session: *const SbjSession,
    json: *const c_char,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    let outputs: Vec<OutputDoc> = match unsafe { required_str(json, "json") }.and_then(|j| {
        serde_json::from_str(j)
            .map_err(|e| fail(SbjStatus::InvalidArgument, format!("bad outputs: {e}")))
    }) {
        Ok(o) => o,
        Err(status) => return status,
    };
    // SAFETY: forwarded caller contract.
    unsafe { with_session(session, |s| session_status(s.lock().set_outputs(outputs))) }
}

// ---------------------------------------------------------------------
// Cues
// ---------------------------------------------------------------------

/// Decodes any of `files` not yet loaded, without holding the session
/// lock, then adds them. Returns the files that failed, with reasons.
fn preload(s: &SbjSession, files: Vec<String>) -> DecodeReport {
    let (missing, dir, rates) = {
        let session = s.lock();
        let missing: Vec<String> = files
            .into_iter()
            .filter(|f| !session.has_sound(f))
            .collect();
        let dir = session
            .show_path()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        (missing, dir, session.output_rates())
    };
    if missing.is_empty() {
        return DecodeReport::default();
    }
    let mut report = decode_files(&missing, dir.as_deref(), &rates);
    let mut session = s.lock();
    for (file, sound) in report.sounds.drain(..) {
        session.add_sound(file, sound);
    }
    report
}

/// Adds or replaces a cue from JSON (one show-format cue object). Its
/// audio is decoded first if needed (slow; call off the main thread).
/// Files that fail to decode are reported in the status's `problems`.
///
/// # Safety
/// `session` must be null or a live session; `json` a valid string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_set_cue_json(
    session: *const SbjSession,
    json: *const c_char,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    let cue: CueDoc = match unsafe { required_str(json, "json") }.and_then(|j| {
        serde_json::from_str(j)
            .map_err(|e| fail(SbjStatus::InvalidArgument, format!("bad cue: {e}")))
    }) {
        Ok(c) => c,
        Err(status) => return status,
    };
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            preload(s, cue.clips.iter().map(|c| c.file.clone()).collect());
            session_status(s.lock().set_cue(cue))
        })
    }
}

/// Decodes an audio file into the session's library so it can be shown
/// (duration, waveform) before it is used by a cue. Slow; call off the
/// main thread. Writes the duration to `duration_out` if non-null.
///
/// # Safety
/// `session` must be null or a live session; `file` a valid string;
/// `duration_out` null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_load_sound(
    session: *const SbjSession,
    file: *const c_char,
    duration_out: *mut f64,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    let file = match unsafe { required_str(file, "file") } {
        Ok(f) => f.to_owned(),
        Err(status) => return status,
    };
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            let report = preload(s, vec![file.clone()]);
            if let Some((_, why)) = report.failures.first() {
                return fail(SbjStatus::MediaError, format!("{file}: {why}"));
            }
            let Some(buffer) = s.lock().sound_buffer(&file) else {
                return fail(SbjStatus::MediaError, format!("{file}: not loaded"));
            };
            if !duration_out.is_null() {
                // SAFETY: guaranteed by the caller.
                *duration_out = buffer.frames() as f64 / f64::from(buffer.sample_rate());
            }
            SbjStatus::Ok
        })
    }
}

/// Fills `out` with `buckets` (min, max) pairs (so `2 * buckets` floats)
/// describing a loaded sound's waveform. The file must have been loaded
/// (by a cue or `sbj_session_load_sound`).
///
/// # Safety
/// `session` must be null or a live session; `file` a valid string; `out`
/// valid for `2 * buckets` floats.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_waveform(
    session: *const SbjSession,
    file: *const c_char,
    buckets: u32,
    out: *mut f32,
) -> SbjStatus {
    if out.is_null() {
        return fail(SbjStatus::NullPointer, "out is null");
    }
    // SAFETY: forwarded caller contract.
    let file = match unsafe { required_str(file, "file") } {
        Ok(f) => f.to_owned(),
        Err(status) => return status,
    };
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            // Grab the audio, then compute without holding the lock.
            let Some(buffer) = s.lock().sound_buffer(&file) else {
                return fail(SbjStatus::MediaError, format!("{file}: not loaded"));
            };
            let peaks = waveform(&buffer, buckets as usize);
            // SAFETY: caller guarantees room for 2 * buckets floats.
            let dst = std::slice::from_raw_parts_mut(out, 2 * buckets as usize);
            for (pair, (lo, hi)) in dst.chunks_exact_mut(2).zip(peaks) {
                pair[0] = lo;
                pair[1] = hi;
            }
            SbjStatus::Ok
        })
    }
}

macro_rules! cue_fn {
    ($(#[$doc:meta])* $name:ident($($arg:ident: $ty:ty),*) => |$s:ident, $cue:ident| $body:expr) => {
        $(#[$doc])*
        ///
        /// # Safety
        /// `session` must be null or a live session.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(
            session: *const SbjSession,
            $cue: u32,
            $($arg: $ty),*
        ) -> SbjStatus {
            // SAFETY: forwarded caller contract.
            unsafe {
                with_session(session, |sess| {
                    let mut $s = sess.lock();
                    session_status($body)
                })
            }
        }
    };
}

cue_fn!(
    /// Empties a pad.
    sbj_session_remove_cue() => |s, slot| s.remove_cue(slot));
cue_fn!(
    /// Moves a cue to pad `to`, swapping with whatever is there.
    sbj_session_move_cue(to: u32) => |s, slot| s.move_cue(slot, to));
cue_fn!(
    /// Plays a cue from the start, restarting it if already playing.
    /// Firing an empty pad does nothing.
    sbj_session_fire_cue() => |s, slot| s.fire_cue(slot));
cue_fn!(
    /// Plays a pad on the preview output (e.g. headphones) without
    /// affecting the main outputs; previewing the same pad again stops it.
    sbj_session_preview_cue() => |s, slot| s.preview_cue(slot));
cue_fn!(
    /// Moves a playing pad's playhead to `seconds` (scrubbing); does
    /// nothing if it isn't playing.
    sbj_session_seek_cue(seconds: f64) => |s, slot| s.seek_cue(slot, seconds));
cue_fn!(
    /// Playlist pad: crossfade to the next song (or start, if stopped).
    sbj_session_skip_cue() => |s, slot| s.skip_cue(slot));
cue_fn!(
    /// Plays a cue from `seconds` into its timeline.
    sbj_session_play_cue_from(seconds: f64) => |s, slot| s.play_cue_from(slot, seconds));
cue_fn!(
    /// Stops a cue, fading out over `fade_seconds` (0 = immediately).
    sbj_session_stop_cue(fade_seconds: f64) => |s, slot| s.stop_cue(slot, fade_seconds));

// ---------------------------------------------------------------------
// Play log
// ---------------------------------------------------------------------

/// Directory for the daily play-log files (`YYYY-MM-DD.jsonl`, UTC days);
/// null turns file logging off.
///
/// # Safety
/// `session` must be null or a live session; `dir` null or a valid string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_set_log_dir(
    session: *const SbjSession,
    dir: *const c_char,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    let dir = match unsafe { read_str(dir) } {
        Ok(d) => d.map(PathBuf::from),
        Err(status) => return status,
    };
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            s.lock().set_log_dir(dir);
            SbjStatus::Ok
        })
    }
}

/// Names the current game; stamped on new log entries.
///
/// # Safety
/// `session` must be null or a live session; `name` a valid string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_set_game(
    session: *const SbjSession,
    name: *const c_char,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    let name = match unsafe { required_str(name, "name") } {
        Ok(n) => n.to_owned(),
        Err(status) => return status,
    };
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            s.lock().set_game(&name);
            SbjStatus::Ok
        })
    }
}

/// The play log as JSON: `{"entries": [{"started" (unix seconds),
/// "seconds", "game", "show", "slot", "bank", "pad", "title", "file",
/// "sponsor", "source", "output", "open"}], "songs": [{"name", "plays",
/// "seconds"}], "sponsors": [...]}`. Free with `sbj_string_free`.
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_log_json(session: *const SbjSession) -> *mut c_char {
    // SAFETY: forwarded caller contract.
    unsafe {
        session_string(session, |s| {
            let session = s.lock();
            let (songs, sponsors) = session.log_totals();
            Ok(json!({
                "entries": session.log_entries(),
                "songs": songs,
                "sponsors": sponsors,
            })
            .to_string())
        })
    }
}

/// Clears the in-memory log (log files are kept).
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_clear_log(session: *const SbjSession) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            s.lock().clear_log();
            SbjStatus::Ok
        })
    }
}

/// Stops any preview.
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_stop_preview(session: *const SbjSession) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            s.lock().stop_preview();
            SbjStatus::Ok
        })
    }
}

/// Chooses the output (index) used for previews; negative turns preview
/// off.
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_set_preview_output(
    session: *const SbjSession,
    output: i32,
) -> SbjStatus {
    let output = u32::try_from(output).ok();
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            session_status(s.lock().set_preview_output(output))
        })
    }
}

/// Stops every cue on every output, fading out over `fade_seconds`.
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_stop_all(
    session: *const SbjSession,
    fade_seconds: f64,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    unsafe { with_session(session, |s| session_status(s.lock().stop_all(fade_seconds))) }
}

/// Sets the master gain (linear) on every output.
///
/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_set_master_gain(
    session: *const SbjSession,
    gain: f32,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    unsafe { with_session(session, |s| session_status(s.lock().set_master_gain(gain))) }
}

/// Fills `out` with the state of `count` consecutive pads starting at
/// `first` (one lock for the whole grid, for 30 Hz UI refresh).
///
/// # Safety
/// `session` must be null or a live session; `out` valid for `count`
/// entries.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_cue_states(
    session: *const SbjSession,
    first: u32,
    count: u32,
    out: *mut SbjCueState,
) -> SbjStatus {
    if out.is_null() {
        return fail(SbjStatus::NullPointer, "out is null");
    }
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            let session = s.lock();
            // SAFETY: caller guarantees room for `count` entries.
            let dst = std::slice::from_raw_parts_mut(out, count as usize);
            for (i, slot) in dst.iter_mut().enumerate() {
                let st = match session.cue_state(first + i as u32) {
                    Ok(st) => st,
                    Err(e) => return session_status(Err(e)),
                };
                *slot = SbjCueState {
                    assigned: st.assigned,
                    playing: st.playing,
                    looping: st.looping,
                    ready: st.ready,
                    holding: st.holding,
                    item: st.item.unwrap_or(u32::MAX),
                    next_item: st.next_item.unwrap_or(u32::MAX),
                    position_seconds: st.position_seconds,
                    duration_seconds: st.duration_seconds,
                };
            }
            SbjStatus::Ok
        })
    }
}

#[cfg(test)]
#[allow(clippy::undocumented_unsafe_blocks)] // tests exercise the C API directly
mod tests {
    use super::*;

    fn last_error() -> String {
        let mut buf = [0 as c_char; 256];
        unsafe { sbj_last_error_message(buf.as_mut_ptr(), buf.len()) };
        unsafe { CStr::from_ptr(buf.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    }

    fn take(s: *mut c_char) -> String {
        assert!(!s.is_null(), "{}", last_error());
        let text = unsafe { CStr::from_ptr(s) }.to_str().unwrap().to_owned();
        unsafe { sbj_string_free(s) };
        text
    }

    #[test]
    fn version_is_crate_version() {
        let v = unsafe { CStr::from_ptr(sbj_version()) };
        assert_eq!(v.to_str().unwrap(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn show_round_trip_without_devices() {
        unsafe {
            let s = sbj_session_create();
            assert!(!s.is_null());
            assert_eq!(sbj_session_open_outputs(s), SbjStatus::Ok);

            let cue = cr#"{"slot":2,"title":"Horn","clips":[{"file":"/nope.wav"}]}"#;
            assert_eq!(sbj_session_set_cue_json(s, cue.as_ptr()), SbjStatus::Ok);
            let show: serde_json::Value =
                serde_json::from_str(&take(sbj_session_show_json(s))).unwrap();
            assert_eq!(show["cues"][0]["title"], "Horn");

            let status: serde_json::Value =
                serde_json::from_str(&take(sbj_session_status_json(s))).unwrap();
            assert_eq!(status["dirty"], true);
            // Works with or without an audio device (Linux CI has none;
            // macOS CI runners have one).
            let has_device = status["outputs"][0]["status"] == "running";
            let problems = status["problems"].as_array().unwrap();
            let mut states = [std::mem::zeroed::<SbjCueState>(); 3];
            assert_eq!(
                sbj_session_cue_states(s, 0, 3, states.as_mut_ptr()),
                SbjStatus::Ok
            );
            assert!(states[2].assigned);
            if has_device {
                assert!(states[2].ready);
                assert!(
                    problems[0]["message"]
                        .as_str()
                        .unwrap()
                        .contains("nope.wav")
                );
                assert_eq!(sbj_session_set_master_gain(s, 0.0), SbjStatus::Ok);
                assert_eq!(sbj_session_fire_cue(s, 2), SbjStatus::Ok);
            } else {
                assert_eq!(status["outputs"][0]["status"], "offline");
                assert!(!states[2].ready);
                assert!(problems.is_empty(), "not loaded: no output");
                assert_eq!(sbj_session_fire_cue(s, 2), SbjStatus::NoOutput);
            }
            assert_eq!(sbj_session_fire_cue(s, 0), SbjStatus::Ok, "empty pad");
            assert_eq!(sbj_session_fire_cue(s, 999), SbjStatus::InvalidArgument);
            assert_eq!(sbj_session_move_cue(s, 2, 5), SbjStatus::Ok);
            assert_eq!(sbj_session_remove_cue(s, 5), SbjStatus::Ok);

            let outputs = cr#"[{"name":"Main"},{"name":"Locker"}]"#;
            assert_eq!(
                sbj_session_set_outputs_json(s, outputs.as_ptr()),
                SbjStatus::Ok
            );
            assert_eq!(
                sbj_session_set_outputs_json(s, c"[]".as_ptr()),
                SbjStatus::ShowError
            );
            assert_eq!(
                sbj_session_set_cue_json(s, c"{nope".as_ptr()),
                SbjStatus::InvalidArgument
            );
            assert!(last_error().contains("bad cue"));
            assert_eq!(
                sbj_session_save_show(s, std::ptr::null()),
                SbjStatus::InvalidArgument,
                "no file yet"
            );
            sbj_session_destroy(s);
        }
    }

    #[test]
    fn remote_reaches_the_session() {
        use std::io::{Read, Write};
        use std::net::{TcpListener, TcpStream};
        unsafe {
            let s = sbj_session_create();
            let settings = cr#"{"name":"T","slots":4,"columns":4,"banks":["A","B","C"]}"#;
            assert_eq!(
                sbj_session_set_show_settings_json(s, settings.as_ptr()),
                SbjStatus::Ok
            );
            assert_eq!(
                sbj_session_set_remote(s, 9, c"12ab".as_ptr()),
                SbjStatus::InvalidArgument
            );
            assert_eq!(
                sbj_session_set_remote(s, 9, std::ptr::null()),
                SbjStatus::NullPointer
            );
            let port = TcpListener::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap()
                .port();
            assert_eq!(
                sbj_session_set_remote(s, port, c"4321".as_ptr()),
                SbjStatus::Ok
            );
            let status = || -> serde_json::Value {
                serde_json::from_str(&take(sbj_session_status_json(s))).unwrap()
            };
            assert_eq!(status()["remote_port"], port);

            let call = |request: &str| -> String {
                let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
                stream.write_all(request.as_bytes()).unwrap();
                let mut response = String::new();
                stream.read_to_string(&mut response).unwrap();
                response
            };
            let reply = call("POST /api/bank/3 HTTP/1.1\r\nX-SBJ-PIN: 4321\r\n\r\n");
            assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
            assert!(reply.contains("\"bank\":2"), "{reply}");
            let st = status();
            assert_eq!(st["bank"], 2);
            assert!(st["last_control"].as_str().unwrap().starts_with("Remote "));
            let reply = call("POST /api/fire/99 HTTP/1.1\r\nX-SBJ-PIN: 4321\r\n\r\n");
            assert!(reply.starts_with("HTTP/1.1 404"), "{reply}");

            assert_eq!(
                sbj_session_set_remote(s, 0, std::ptr::null()),
                SbjStatus::Ok
            );
            assert!(status()["remote_port"].is_null());
            assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
            sbj_session_destroy(s);
        }
    }

    #[test]
    fn osc_reaches_the_session() {
        use std::net::UdpSocket;
        unsafe {
            let s = sbj_session_create();
            let settings = cr#"{"name":"T","slots":4,"columns":4,"banks":["A","B","C"]}"#;
            assert_eq!(
                sbj_session_set_show_settings_json(s, settings.as_ptr()),
                SbjStatus::Ok
            );
            // Find a free port, then listen on it.
            let port = UdpSocket::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap()
                .port();
            assert_eq!(sbj_session_set_osc_port(s, port), SbjStatus::Ok);
            let status = || -> serde_json::Value {
                serde_json::from_str(&take(sbj_session_status_json(s))).unwrap()
            };
            assert_eq!(status()["osc_port"], port);

            // A hand-encoded OSC message: "/sbj/bank/3" with no arguments.
            let mut packet = b"/sbj/bank/3\0".to_vec();
            packet.extend_from_slice(b",\0\0\0");
            let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
            sender.send_to(&packet, ("127.0.0.1", port)).unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while status()["bank"] != 2 && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let st = status();
            assert_eq!(st["bank"], 2);
            assert!(st["last_control"].as_str().unwrap().contains("bank 3"));

            assert_eq!(sbj_session_set_osc_port(s, 0), SbjStatus::Ok);
            assert!(status()["osc_port"].is_null());
            let preflight: serde_json::Value =
                serde_json::from_str(&take(sbj_session_preflight_json(s))).unwrap();
            assert!(preflight["checks"].as_array().is_some());
            sbj_session_destroy(s);
        }
    }

    #[test]
    fn null_and_bad_arguments() {
        unsafe {
            assert_eq!(
                sbj_session_fire_cue(std::ptr::null(), 0),
                SbjStatus::NullPointer
            );
            assert!(sbj_session_show_json(std::ptr::null()).is_null());
            sbj_session_destroy(std::ptr::null_mut());
            sbj_string_free(std::ptr::null_mut());
            let s = sbj_session_create();
            assert_eq!(
                sbj_session_open_show(s, c"/definitely/not/here.sbjshow".as_ptr()),
                SbjStatus::ShowError
            );
            assert!(
                last_error().contains("could not read show"),
                "{}",
                last_error()
            );
            let mut d = 0.0;
            assert_eq!(
                sbj_session_load_sound(s, c"/nope.wav".as_ptr(), &mut d),
                SbjStatus::MediaError
            );
            sbj_session_destroy(s);
        }
    }

    #[test]
    fn copy_str_truncates_on_char_boundary() {
        let mut buf = [1 as c_char; 4];
        assert!(!copy_str("aé€", &mut buf));
        let s = unsafe { CStr::from_ptr(buf.as_ptr()) };
        assert_eq!(s.to_str().unwrap(), "aé");
    }
}
