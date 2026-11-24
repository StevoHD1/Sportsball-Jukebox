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
//!   internal lock that the audio thread never touches. Sounds are
//!   immutable once loaded and may be shared freely.

use std::cell::RefCell;
use std::ffi::{CStr, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Mutex, MutexGuard};

use sbj_backend::system::SystemBackend;
use sbj_backend::{AudioBackend, DeviceId, DeviceInfo, StreamHealth};
use sbj_engine::EngineError;
use sbj_session::{Session, SessionError, Sound};

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
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SbjOutputHealth {
    Running = 0,
    DeviceLost = 1,
    Failed = 2,
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
pub struct SbjOutputInfo {
    pub is_open: bool,
    pub device_id: [c_char; SBJ_ID_LEN],
    pub sample_rate: u32,
    pub channels: u16,
    pub health: SbjOutputHealth,
    pub xruns: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SbjCueState {
    pub assigned: bool,
    pub playing: bool,
    pub looping: bool,
    pub gain: f32,
    pub position_seconds: f64,
    pub duration_seconds: f64,
}

/// Opaque session handle.
pub struct SbjSession {
    session: Mutex<Session<SystemBackend>>,
    /// Snapshot from the last `sbj_session_device_count`, so indices stay
    /// stable while the UI reads them.
    devices: Mutex<Vec<DeviceInfo>>,
}

impl SbjSession {
    fn lock(&self) -> MutexGuard<'_, Session<SystemBackend>> {
        // A panic while locked was already reported; the session's state
        // is still consistent (every mutation is a single assignment or an
        // engine command), so keep going rather than failing forever.
        self.session.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Opaque decoded sound.
pub struct SbjSound(Sound);

// ---------------------------------------------------------------------
// Error reporting
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
                SessionError::InvalidCue(_) => SbjStatus::InvalidArgument,
                SessionError::NoOutput => SbjStatus::NoOutput,
                SessionError::Backend(_) => SbjStatus::DeviceError,
                SessionError::Media(_) => SbjStatus::MediaError,
                SessionError::Engine(EngineError::QueueFull) => SbjStatus::QueueFull,
                SessionError::Engine(_) | SessionError::Timeline(_) => SbjStatus::InternalError,
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
pub extern "C" fn sbj_abi_check(
    device_info_size: usize,
    output_info_size: usize,
    cue_state_size: usize,
) -> bool {
    device_info_size == std::mem::size_of::<SbjDeviceInfo>()
        && output_info_size == std::mem::size_of::<SbjOutputInfo>()
        && cue_state_size == std::mem::size_of::<SbjCueState>()
}

// ---------------------------------------------------------------------
// Sounds
// ---------------------------------------------------------------------

/// Decodes an audio file into memory. Slow (up to a second for long
/// files): call it off the main thread. Pass the open output's sample
/// rate as `target_sample_rate` (or 0 if unknown) so resampling also
/// happens here rather than when the sound is assigned.
///
/// Returns null on failure, with `*status_out` (if non-null) set and the
/// reason in `sbj_last_error_message`. Free with `sbj_sound_free`.
///
/// # Safety
/// `path` must be a valid NUL-terminated UTF-8 string; `status_out` must be
/// null or valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_sound_load(
    path: *const c_char,
    target_sample_rate: u32,
    status_out: *mut SbjStatus,
) -> *mut SbjSound {
    let mut result = std::ptr::null_mut();
    let status = guard(|| {
        // SAFETY: forwarded caller contract.
        let path = match unsafe { read_str(path) } {
            Ok(Some(p)) => p,
            Ok(None) => return fail(SbjStatus::NullPointer, "path is null"),
            Err(s) => return s,
        };
        let target = (target_sample_rate > 0).then_some(target_sample_rate);
        match Sound::load(path, target) {
            Ok(sound) => {
                result = Box::into_raw(Box::new(SbjSound(sound)));
                SbjStatus::Ok
            }
            Err(e) => session_status(Err(e)),
        }
    });
    if !status_out.is_null() {
        // SAFETY: guaranteed by the caller.
        unsafe { *status_out = status };
    }
    result
}

/// Frees a sound. Cues it was assigned to keep their own reference.
///
/// # Safety
/// `sound` must be null or from `sbj_sound_load`, not already freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_sound_free(sound: *mut SbjSound) {
    if !sound.is_null() {
        // SAFETY: the pointer came from `Box::into_raw`.
        drop(unsafe { Box::from_raw(sound) });
    }
}

/// # Safety
/// `sound` must be null or a live sound.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_sound_duration_seconds(sound: *const SbjSound) -> f64 {
    // SAFETY: guaranteed by the caller.
    unsafe { sound.as_ref() }.map_or(0.0, |s| s.0.duration_seconds())
}

/// # Safety
/// `sound` must be null or a live sound.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_sound_channels(sound: *const SbjSound) -> u16 {
    // SAFETY: guaranteed by the caller.
    unsafe { sound.as_ref() }.map_or(0, |s| s.0.channels())
}

// ---------------------------------------------------------------------
// Session lifecycle and devices
// ---------------------------------------------------------------------

/// Creates a session with `cue_count` cue slots (1..=1024). No output is
/// open yet. Returns null on invalid arguments.
#[unsafe(no_mangle)]
pub extern "C" fn sbj_session_create(cue_count: u32) -> *mut SbjSession {
    if !(1..=1024).contains(&cue_count) {
        set_error("cue count must be between 1 and 1024");
        return std::ptr::null_mut();
    }
    catch_unwind(|| {
        Box::into_raw(Box::new(SbjSession {
            session: Mutex::new(Session::new(SystemBackend::new(), cue_count)),
            devices: Mutex::new(Vec::new()),
        }))
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Closes the output and frees the session. Null is ignored.
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
            let devices = match s.lock().devices() {
                Ok(d) => d,
                Err(e) => return session_status(Err(e)),
            };
            count = devices.len() as u32;
            *s.devices.lock().unwrap_or_else(|e| e.into_inner()) = devices;
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
            let devices = s.devices.lock().unwrap_or_else(|e| e.into_inner());
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

/// Opens an output device by id (from `sbj_session_device_info`), or the
/// system default if `device_id` is null. Replaces any open output.
///
/// # Safety
/// `session` must be null or a live session; `device_id` null or a valid
/// NUL-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_open_output(
    session: *const SbjSession,
    device_id: *const c_char,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    let id = match unsafe { read_str(device_id) } {
        Ok(id) => id.map(|s| DeviceId(s.to_owned())),
        Err(status) => return status,
    };
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            session_status(s.lock().open_output(id.as_ref()))
        })
    }
}

/// # Safety
/// `session` must be null or a live session.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_close_output(session: *const SbjSession) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            s.lock().close_output();
            SbjStatus::Ok
        })
    }
}

/// # Safety
/// `session` must be null or a live session; `out` null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_output_info(
    session: *const SbjSession,
    out: *mut SbjOutputInfo,
) -> SbjStatus {
    // SAFETY: guaranteed by the caller.
    let Some(out) = (unsafe { out.as_mut() }) else {
        return fail(SbjStatus::NullPointer, "out is null");
    };
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            match s.lock().output_state() {
                None => {
                    out.is_open = false;
                    out.device_id[0] = 0;
                    out.sample_rate = 0;
                    out.channels = 0;
                    out.health = SbjOutputHealth::Running;
                    out.xruns = 0;
                }
                Some(o) => {
                    out.is_open = true;
                    copy_str(&o.device.0, &mut out.device_id);
                    out.sample_rate = o.sample_rate;
                    out.channels = o.channels;
                    out.health = match o.health {
                        StreamHealth::Running => SbjOutputHealth::Running,
                        StreamHealth::DeviceLost => SbjOutputHealth::DeviceLost,
                        StreamHealth::Failed => SbjOutputHealth::Failed,
                    };
                    out.xruns = o.xruns;
                }
            }
            SbjStatus::Ok
        })
    }
}

/// Housekeeping: drains engine events and frees retired audio. Call on the
/// UI refresh timer.
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

// ---------------------------------------------------------------------
// Cues
// ---------------------------------------------------------------------

/// Assigns a sound to a cue (the session keeps its own reference; the
/// caller still frees `sound`). Stops the cue if it was playing.
///
/// # Safety
/// `session` and `sound` must be null or live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_set_cue_sound(
    session: *const SbjSession,
    cue: u32,
    sound: *const SbjSound,
) -> SbjStatus {
    // SAFETY: guaranteed by the caller.
    let Some(sound) = (unsafe { sound.as_ref() }) else {
        return fail(SbjStatus::NullPointer, "sound is null");
    };
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| {
            session_status(s.lock().set_cue_sound(cue, sound.0.clone()))
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
    /// Empties a cue.
    sbj_session_clear_cue() => |s, cue| s.clear_cue(cue));
cue_fn!(
    /// Plays a cue from the start, restarting it if already playing.
    sbj_session_fire_cue() => |s, cue| s.fire_cue(cue));
cue_fn!(
    /// Stops a cue, fading out over `fade_seconds` (0 = immediately).
    sbj_session_stop_cue(fade_seconds: f64) => |s, cue| s.stop_cue(cue, fade_seconds));
cue_fn!(
    /// Turns looping on or off (applies on next fire if playing).
    sbj_session_set_cue_loop(looping: bool) => |s, cue| s.set_cue_loop(cue, looping));
cue_fn!(
    /// Sets a cue's gain (linear; 1.0 = unchanged).
    sbj_session_set_cue_gain(gain: f32) => |s, cue| s.set_cue_gain(cue, gain));

/// Stops every cue, fading out over `fade_seconds` (0 = immediately).
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

/// Sets the master output gain (linear).
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

/// # Safety
/// `session` must be null or a live session; `out` null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_session_cue_state(
    session: *const SbjSession,
    cue: u32,
    out: *mut SbjCueState,
) -> SbjStatus {
    // SAFETY: guaranteed by the caller.
    let Some(out) = (unsafe { out.as_mut() }) else {
        return fail(SbjStatus::NullPointer, "out is null");
    };
    // SAFETY: forwarded caller contract.
    unsafe {
        with_session(session, |s| match s.lock().cue_state(cue) {
            Ok(st) => {
                *out = SbjCueState {
                    assigned: st.assigned,
                    playing: st.playing,
                    looping: st.looping,
                    gain: st.gain,
                    position_seconds: st.position_seconds,
                    duration_seconds: st.duration_seconds,
                };
                SbjStatus::Ok
            }
            Err(e) => session_status(Err(e)),
        })
    }
}

/// Name of the audio system in use ("CoreAudio" on macOS).
///
/// # Safety
/// `session` must be null or a live session.
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

    #[test]
    fn version_is_crate_version() {
        let v = unsafe { CStr::from_ptr(sbj_version()) };
        assert_eq!(v.to_str().unwrap(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn session_without_output() {
        unsafe {
            let s = sbj_session_create(4);
            assert!(!s.is_null());
            // CI machines have no audio devices: firing must fail cleanly.
            assert_eq!(
                sbj_session_fire_cue(s, 0),
                SbjStatus::Ok,
                "empty cue is a no-op"
            );
            assert_eq!(sbj_session_stop_all(s, 0.5), SbjStatus::NoOutput);
            assert!(last_error().contains("no audio output"));
            assert_eq!(sbj_session_fire_cue(s, 99), SbjStatus::InvalidArgument);

            let mut st = std::mem::zeroed::<SbjCueState>();
            assert_eq!(sbj_session_cue_state(s, 1, &mut st), SbjStatus::Ok);
            assert!(!st.assigned && st.gain == 1.0);

            let mut info = std::mem::zeroed::<SbjOutputInfo>();
            assert_eq!(sbj_session_output_info(s, &mut info), SbjStatus::Ok);
            assert!(!info.is_open);
            sbj_session_destroy(s);
        }
    }

    #[test]
    fn null_and_bad_arguments() {
        unsafe {
            assert!(sbj_session_create(0).is_null());
            assert_eq!(
                sbj_session_fire_cue(std::ptr::null(), 0),
                SbjStatus::NullPointer
            );
            sbj_session_destroy(std::ptr::null_mut());
            sbj_sound_free(std::ptr::null_mut());
            assert_eq!(sbj_sound_duration_seconds(std::ptr::null()), 0.0);

            let mut status = SbjStatus::Ok;
            let missing = sbj_sound_load(c"/definitely/not/here.wav".as_ptr(), 0, &mut status);
            assert!(missing.is_null());
            assert_eq!(status, SbjStatus::MediaError);
            assert!(
                last_error().contains("could not read file"),
                "{}",
                last_error()
            );
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
