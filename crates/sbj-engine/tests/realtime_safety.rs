//! Enforces the core real-time rule: `Renderer::process` never allocates or
//! frees memory. A counting global allocator flags any heap activity on the
//! thread while it is inside `process`.
//!
//! This file is its own test binary, so the custom allocator does not affect
//! other tests.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::*;
use sbj_engine::{Clip, LoopCount, Marker, MarkerAction, Renderer, Timeline};

struct GuardedAlloc;

static VIOLATIONS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static IN_AUDIO_CALLBACK: Cell<bool> = const { Cell::new(false) };
}

fn flag() {
    if IN_AUDIO_CALLBACK.try_with(Cell::get).unwrap_or(false) {
        VIOLATIONS.fetch_add(1, Ordering::SeqCst);
    }
}

// SAFETY: forwards to the system allocator unchanged; only adds counting.
unsafe impl GlobalAlloc for GuardedAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        flag();
        // SAFETY: forwarded contract.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        flag();
        // SAFETY: forwarded contract.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        flag();
        // SAFETY: forwarded contract.
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        flag();
        // SAFETY: forwarded contract.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: GuardedAlloc = GuardedAlloc;

fn guarded_process(r: &mut Renderer, buf: &mut [f32]) {
    IN_AUDIO_CALLBACK.with(|f| f.set(true));
    r.process(buf);
    IN_AUDIO_CALLBACK.with(|f| f.set(false));
}

#[test]
fn process_never_allocates_or_frees() {
    let busy = Timeline::builder(RATE)
        .clip(Clip::new(ramp_buffer(10_000)))
        .clip(Clip::new(ramp_buffer(5_000)).at(2_000))
        .marker(Marker::new(
            1,
            3_000,
            MarkerAction::Loop {
                to: 1_000,
                count: LoopCount::Times(3),
            },
        ))
        .marker(Marker::new(2, 6_000, MarkerAction::Jump { to: 8_000 }))
        .marker(Marker::new(3, 9_000, MarkerAction::FadeOut { length: 500 }))
        .build()
        .unwrap();

    let (mut c, mut r) = engine(2);
    let mut buf = vec![0.0f32; 256 * 2];

    // Exercise every command path, including swapping timelines while
    // playing (the old timeline must be freed on this thread, not in
    // `process`).
    c.load_timeline(busy.clone()).unwrap();
    c.set_master_gain(0.8).unwrap();
    c.play().unwrap();
    for i in 0..200 {
        match i {
            20 => c.seek(500).unwrap(),
            40 => c.pause().unwrap(),
            45 => c.play().unwrap(),
            60 => c.load_timeline(busy.clone()).unwrap(),
            61 => c.play().unwrap(),
            100 => c.fade_out_and_stop(1_000).unwrap(),
            120 => c.play().unwrap(),
            150 => c.stop().unwrap(),
            _ => {}
        }
        guarded_process(&mut r, &mut buf);
        while c.poll_event().is_some() {}
    }

    assert_eq!(VIOLATIONS.load(Ordering::SeqCst), 0);
    assert_eq!(c.leaked_garbage(), 0);
    assert_eq!(c.dropped_events(), 0);
}
