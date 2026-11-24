//! C ABI over the engine, consumed by the Swift app (and by a future
//! Windows app). `include/sportsball_jukebox.h` must be kept in sync with
//! this file; `tests/smoke.c` compiles against the header in CI to catch
//! drift.
//!
//! Conventions:
//! * Every function is panic-safe: a panic is caught and reported as
//!   `SBJ_STATUS_INTERNAL_ERROR` rather than unwinding into Swift.
//! * Null handles are tolerated and reported as `SBJ_STATUS_NULL_POINTER`.
//! * Phase 0: the handle owns both controller and renderer, and audio is
//!   pulled with `sbj_engine_render`. Phase 1 moves the renderer into a
//!   Core Audio stream owned by the handle.

use std::ffi::{CStr, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use sbj_engine::{AudioBuffer, Clip, Controller, Engine, EngineConfig, Renderer, Timeline};

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SbjStatus {
    Ok = 0,
    NullPointer = 1,
    InvalidArgument = 2,
    QueueFull = 3,
    InternalError = 4,
}

/// Opaque engine handle.
pub struct SbjEngine {
    controller: Controller,
    renderer: Renderer,
}

fn guard<F: FnOnce() -> SbjStatus>(f: F) -> SbjStatus {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(SbjStatus::InternalError)
}

fn status<E>(r: Result<(), E>) -> SbjStatus {
    match r {
        Ok(()) => SbjStatus::Ok,
        Err(_) => SbjStatus::QueueFull,
    }
}

/// # Safety
/// `engine` must be null or a live handle from `sbj_engine_create`, not used
/// concurrently from another thread.
unsafe fn with_engine(
    engine: *mut SbjEngine,
    f: impl FnOnce(&mut SbjEngine) -> SbjStatus,
) -> SbjStatus {
    // SAFETY: guaranteed by the caller per this function's contract.
    match unsafe { engine.as_mut() } {
        Some(e) => guard(|| f(e)),
        None => SbjStatus::NullPointer,
    }
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

/// Creates an engine. Returns null on invalid arguments.
#[unsafe(no_mangle)]
pub extern "C" fn sbj_engine_create(sample_rate: u32, channels: u16) -> *mut SbjEngine {
    catch_unwind(
        || match Engine::create(EngineConfig::new(sample_rate, channels)) {
            Ok((controller, renderer)) => Box::into_raw(Box::new(SbjEngine {
                controller,
                renderer,
            })),
            Err(_) => std::ptr::null_mut(),
        },
    )
    .unwrap_or(std::ptr::null_mut())
}

/// Destroys an engine. Null is ignored.
///
/// # Safety
/// `engine` must be null or a handle from `sbj_engine_create` that has not
/// already been destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_engine_destroy(engine: *mut SbjEngine) {
    if !engine.is_null() {
        // SAFETY: the pointer came from `Box::into_raw` in `sbj_engine_create`.
        drop(unsafe { Box::from_raw(engine) });
    }
}

/// Loads a timeline containing a single sine-wave clip. A stand-in until
/// file loading exists, so the first Mac build can make a sound.
///
/// # Safety
/// See `with_engine`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_engine_load_test_tone(
    engine: *mut SbjEngine,
    frequency_hz: f32,
    seconds: f32,
) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    unsafe {
        with_engine(engine, |e| {
            if !(frequency_hz.is_finite() && frequency_hz > 0.0)
                || !(seconds.is_finite() && seconds > 0.0 && seconds <= 600.0)
            {
                return SbjStatus::InvalidArgument;
            }
            let rate = e.controller.config().sample_rate;
            let frames = (seconds * rate as f32) as usize;
            let step = std::f32::consts::TAU * frequency_hz / rate as f32;
            let data = (0..frames)
                .map(|i| 0.25 * (step * i as f32).sin())
                .collect();
            let Ok(buffer) = AudioBuffer::from_interleaved(rate, 1, data) else {
                return SbjStatus::InvalidArgument;
            };
            let Ok(timeline) = Timeline::builder(rate)
                .clip(Clip::new(Arc::new(buffer)))
                .build()
            else {
                return SbjStatus::InvalidArgument;
            };
            status(e.controller.load_timeline(timeline))
        })
    }
}

macro_rules! transport_fn {
    ($(#[$doc:meta])* $name:ident, $method:ident) => {
        $(#[$doc])*
        ///
        /// # Safety
        /// See `with_engine`.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(engine: *mut SbjEngine) -> SbjStatus {
            // SAFETY: forwarded caller contract.
            unsafe { with_engine(engine, |e| status(e.controller.$method())) }
        }
    };
}

transport_fn!(
    /// Starts or resumes playback.
    sbj_engine_play, play);
transport_fn!(
    /// Halts playback, keeping the playhead position.
    sbj_engine_pause, pause);
transport_fn!(
    /// Halts playback and rewinds to the start.
    sbj_engine_stop, stop);

/// Moves the playhead to `position` (in sample frames).
///
/// # Safety
/// See `with_engine`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_engine_seek(engine: *mut SbjEngine, position: u64) -> SbjStatus {
    // SAFETY: forwarded caller contract.
    unsafe { with_engine(engine, |e| status(e.controller.seek(position))) }
}

/// Playhead position in sample frames, or 0 for a null handle.
///
/// # Safety
/// See `with_engine`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_engine_position(engine: *const SbjEngine) -> u64 {
    // SAFETY: forwarded caller contract.
    unsafe { engine.as_ref() }.map_or(0, |e| e.controller.position())
}

/// Renders `frames` interleaved frames into `out`, which must hold
/// `frames * channels` floats.
///
/// # Safety
/// See `with_engine`; `out` must be valid for writes of that many floats.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sbj_engine_render(
    engine: *mut SbjEngine,
    out: *mut f32,
    frames: usize,
) -> SbjStatus {
    if out.is_null() {
        return SbjStatus::NullPointer;
    }
    // SAFETY: forwarded caller contract.
    unsafe {
        with_engine(engine, |e| {
            let Some(len) = frames.checked_mul(e.renderer.channels()) else {
                return SbjStatus::InvalidArgument;
            };
            // SAFETY: caller guarantees `out` holds `frames * channels` floats.
            let buf = std::slice::from_raw_parts_mut(out, len);
            e.renderer.process(buf);
            SbjStatus::Ok
        })
    }
}

#[cfg(test)]
#[allow(clippy::undocumented_unsafe_blocks)] // tests exercise the C API directly
mod tests {
    use super::*;

    #[test]
    fn plays_test_tone_through_c_api() {
        unsafe {
            let e = sbj_engine_create(48_000, 2);
            assert!(!e.is_null());
            assert_eq!(sbj_engine_load_test_tone(e, 440.0, 0.1), SbjStatus::Ok);
            assert_eq!(sbj_engine_play(e), SbjStatus::Ok);
            let mut out = vec![0.0f32; 512 * 2];
            assert_eq!(sbj_engine_render(e, out.as_mut_ptr(), 512), SbjStatus::Ok);
            assert_eq!(sbj_engine_position(e), 512);
            assert!(out.iter().any(|&s| s != 0.0));
            sbj_engine_destroy(e);
        }
    }

    #[test]
    fn tolerates_null_and_bad_arguments() {
        unsafe {
            assert!(sbj_engine_create(0, 2).is_null());
            assert_eq!(
                sbj_engine_play(std::ptr::null_mut()),
                SbjStatus::NullPointer
            );
            assert_eq!(sbj_engine_position(std::ptr::null()), 0);
            sbj_engine_destroy(std::ptr::null_mut());
            let e = sbj_engine_create(48_000, 2);
            assert_eq!(
                sbj_engine_load_test_tone(e, f32::NAN, 1.0),
                SbjStatus::InvalidArgument
            );
            sbj_engine_destroy(e);
        }
    }

    #[test]
    fn version_is_crate_version() {
        let v = unsafe { CStr::from_ptr(sbj_version()) };
        assert_eq!(v.to_str().unwrap(), env!("CARGO_PKG_VERSION"));
    }
}
