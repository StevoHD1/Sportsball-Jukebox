//! How long loudness analysis takes for a 4-minute stereo song.
//!
//! ```sh
//! cargo run --release -p sbj-media --example analyze_cost
//! ```

use sbj_engine::AudioBuffer;
use std::time::Instant;

fn main() {
    let frames = 48_000 * 240;
    let data: Vec<f32> = (0..frames * 2)
        .map(|i| ((i as f32) * 0.0013).sin() * 0.3)
        .collect();
    let buffer = AudioBuffer::from_interleaved(48_000, 2, data).unwrap();
    let start = Instant::now();
    let analysis = sbj_media::analyze(&buffer);
    println!(
        "4 min stereo: {:?} ({:.1} LUFS)",
        start.elapsed(),
        analysis.loudness_lufs.unwrap_or(f64::NAN)
    );
}
