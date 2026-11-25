#![allow(dead_code)]

use std::sync::Arc;

use sbj_engine::{AudioBuffer, Controller, Engine, EngineConfig, Event, PlayerId, Renderer};

pub const RATE: u32 = 48_000;

/// Block sizes used to prove behaviour does not depend on how the device
/// chops up the stream.
pub const BLOCK_SIZES: &[usize] = &[1, 7, 64, 333, 512];

/// A mono buffer whose sample `i` has value `i + 1`, so rendered output
/// reveals exactly which source frame was played (and 0.0 means silence).
pub fn ramp_buffer(frames: usize) -> Arc<AudioBuffer> {
    let data = (0..frames).map(|i| (i + 1) as f32).collect();
    Arc::new(AudioBuffer::from_interleaved(RATE, 1, data).unwrap())
}

/// Expected output for playing source frames `range` of a ramp buffer.
pub fn ramp(range: std::ops::Range<usize>) -> Vec<f32> {
    range.map(|i| (i + 1) as f32).collect()
}

pub fn engine(channels: u16) -> (Controller, Renderer) {
    let mut config = EngineConfig::new(RATE, channels);
    config.gain_ramp_frames = 0;
    config.declick_frames = 0;
    Engine::create(config).unwrap()
}

/// Renders mono output in `block`-frame chunks until playback stops (or
/// `max_frames` is reached), returning only the frames rendered while
/// playing.
pub fn render_until_stopped(
    controller: &Controller,
    renderer: &mut Renderer,
    block: usize,
    max_frames: usize,
) -> Vec<f32> {
    let mut out = Vec::new();
    let mut buf = vec![0.0; block];
    while out.len() < max_frames {
        renderer.process(&mut buf);
        out.extend_from_slice(&buf);
        if !controller.is_playing(PlayerId(0)) {
            break;
        }
    }
    // Trim trailing silence produced after the stop inside the last block.
    while out.last() == Some(&0.0) {
        out.pop();
    }
    out
}

pub fn render(renderer: &mut Renderer, frames: usize, block: usize) -> Vec<f32> {
    let ch = renderer.channels();
    let mut out = Vec::with_capacity(frames * ch);
    let mut buf = vec![0.0; block * ch];
    let mut left = frames;
    while left > 0 {
        let n = left.min(block);
        renderer.process(&mut buf[..n * ch]);
        out.extend_from_slice(&buf[..n * ch]);
        left -= n;
    }
    out
}

pub fn events(controller: &mut Controller) -> Vec<Event> {
    std::iter::from_fn(|| controller.poll_event()).collect()
}
