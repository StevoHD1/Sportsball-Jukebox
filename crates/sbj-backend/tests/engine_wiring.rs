//! Proves the engine plugs into a backend the same way it will on macOS.

use std::sync::Arc;

use sbj_backend::offline::{OFFLINE_DEVICE, OfflineBackend};
use sbj_backend::{AudioBackend, DeviceId, StreamConfig};
use sbj_engine::{AudioBuffer, Clip, Engine, EngineConfig, Timeline};

#[test]
fn engine_renders_through_offline_backend() {
    let rate = 48_000;
    let (mut controller, mut renderer) = Engine::create(EngineConfig::new(rate, 2)).unwrap();

    let buffer = Arc::new(AudioBuffer::from_interleaved(rate, 1, vec![0.5; 1000]).unwrap());
    let timeline = Timeline::builder(rate)
        .clip(Clip::new(buffer))
        .build()
        .unwrap();
    controller.load_timeline(timeline).unwrap();
    controller.play().unwrap();

    let backend = OfflineBackend;
    let device = &backend.output_devices().unwrap()[0];
    assert_eq!(device.id, DeviceId(OFFLINE_DEVICE.into()));

    let config = StreamConfig {
        sample_rate: rate,
        channels: 2,
        buffer_frames: 256,
    };
    let mut stream = backend
        .open_output(
            &device.id,
            config,
            Box::new(move |out| renderer.process(out)),
        )
        .unwrap();

    let audio = stream.render(1200);
    assert_eq!(audio.len(), 2400);
    assert!(audio[..2000].iter().all(|&s| s == 0.5));
    assert!(audio[2000..].iter().all(|&s| s == 0.0));
    assert!(!controller.is_playing());
}
