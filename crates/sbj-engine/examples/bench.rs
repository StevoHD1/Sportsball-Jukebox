//! Measures how much of the audio deadline the engine uses.
//!
//!     cargo run --release -p sbj-engine --example bench
//!
//! Renders 30 s of audio with many players running at once, in 128-frame
//! blocks (2.7 ms at 48 kHz), and reports the average and worst time per
//! block as a percentage of the time available.

use std::sync::Arc;
use std::time::{Duration, Instant};

use sbj_engine::{
    AudioBuffer, Clip, Engine, EngineConfig, LoopCount, Marker, MarkerAction, PlayerId, Timeline,
};

const RATE: u32 = 48_000;
const BLOCK: usize = 128;
const SECONDS: usize = 30;

fn main() {
    for players in [1, 16, 64, 128] {
        run(players);
    }
}

fn run(players: u32) {
    let mut config = EngineConfig::new(RATE, 2);
    config.players = players;
    let (mut controller, mut renderer) = Engine::create(config).unwrap();

    // A stereo 10 s sound with a loop marker, so markers and crossfades
    // are exercised too.
    let frames = RATE as usize * 10;
    let data: Vec<f32> = (0..frames * 2)
        .map(|i| ((i as f32) * 0.001).sin() * 0.1)
        .collect();
    let buffer = Arc::new(AudioBuffer::from_interleaved(RATE, 2, data).unwrap());
    for p in 0..players {
        let timeline = Timeline::builder(RATE)
            .clip(Clip::new(Arc::clone(&buffer)))
            .marker(Marker::new(
                1,
                RATE as u64 * 9,
                MarkerAction::Loop {
                    to: RATE as u64,
                    count: LoopCount::Infinite,
                },
            ))
            .build()
            .unwrap();
        controller.load(PlayerId(p), timeline).unwrap();
        controller.set_player_gain(PlayerId(p), 0.5).unwrap();
        controller.play(PlayerId(p)).unwrap();
    }

    let mut out = vec![0.0f32; BLOCK * 2];
    let blocks = SECONDS * RATE as usize / BLOCK;
    let budget = Duration::from_secs_f64(BLOCK as f64 / f64::from(RATE));
    let mut total = Duration::ZERO;
    let mut worst = Duration::ZERO;
    for _ in 0..blocks {
        let start = Instant::now();
        renderer.process(&mut out);
        let took = start.elapsed();
        total += took;
        worst = worst.max(took);
        while controller.poll_event().is_some() {}
    }
    let avg = total / blocks as u32;
    println!(
        "{players:>4} players: average {:>6.2}% of deadline ({:?}/block), worst {:>6.2}% ({:?})",
        100.0 * avg.as_secs_f64() / budget.as_secs_f64(),
        avg,
        100.0 * worst.as_secs_f64() / budget.as_secs_f64(),
        worst,
    );
}
