//! End-to-end: session → engine → backend callback, using the offline
//! backend in place of Core Audio.

use sbj_backend::offline::OfflineBackend;
use sbj_backend::{OutputStream, StreamHealth};
use sbj_engine::AudioBuffer;
use sbj_session::{Session, SessionError, Sound};

const RATE: u32 = 48_000;

fn sound(value: f32, frames: usize, rate: u32) -> Sound {
    Sound::from_buffer(
        AudioBuffer::from_interleaved(rate, 1, vec![value; frames]).unwrap(),
        None,
    )
    .unwrap()
}

fn session() -> Session<OfflineBackend> {
    let mut s = Session::new(OfflineBackend::default(), 8);
    s.open_output(None).unwrap();
    s
}

fn pull(s: &mut Session<OfflineBackend>, frames: usize) -> Vec<f32> {
    s.output_stream_mut().unwrap().render(frames)
}

/// Left channel only.
fn left(stereo: &[f32]) -> Vec<f32> {
    stereo.chunks(2).map(|f| f[0]).collect()
}

#[test]
fn fire_plays_cue_to_completion() {
    let mut s = session();
    s.set_cue_sound(0, sound(0.5, 1000, RATE)).unwrap();
    s.fire_cue(0).unwrap();
    let out = pull(&mut s, 1500);
    assert!(out[..2000].iter().all(|&x| x == 0.5), "both channels");
    assert!(out[2000..].iter().all(|&x| x == 0.0));
    let st = s.cue_state(0).unwrap();
    assert!(st.assigned && !st.playing);
}

#[test]
fn cues_overlap() {
    let mut s = session();
    s.set_cue_sound(0, sound(0.25, 1000, RATE)).unwrap();
    s.set_cue_sound(3, sound(0.5, 1000, RATE)).unwrap();
    s.fire_cue(0).unwrap();
    s.fire_cue(3).unwrap();
    assert!(left(&pull(&mut s, 100)).iter().all(|&x| x == 0.75));
    assert!(s.cue_state(0).unwrap().playing && s.cue_state(3).unwrap().playing);
}

#[test]
fn sounds_at_other_rates_are_converted() {
    let mut s = session();
    // One second at 44.1 kHz should play for one second at 48 kHz.
    s.set_cue_sound(0, sound(0.5, 44_100, 44_100)).unwrap();
    s.fire_cue(0).unwrap();
    let out = left(&pull(&mut s, 50_000));
    let audible = out.iter().filter(|x| x.abs() > 0.01).count();
    assert!((audible as i64 - 48_000).abs() < 50, "{audible}");
    assert!((s.cue_state(0).unwrap().duration_seconds - 1.0).abs() < 1e-9);
}

#[test]
fn looping_cue_keeps_going_until_stopped() {
    let mut s = session();
    s.set_cue_sound(1, sound(0.5, 100, RATE)).unwrap();
    s.set_cue_loop(1, true).unwrap();
    s.fire_cue(1).unwrap();
    assert!(left(&pull(&mut s, 1000)).iter().all(|&x| x == 0.5));
    assert!(s.cue_state(1).unwrap().playing);

    s.stop_cue(1, 0.0).unwrap();
    pull(&mut s, 10);
    assert!(!s.cue_state(1).unwrap().playing);
}

#[test]
fn loop_change_while_playing_applies_on_next_fire() {
    let mut s = session();
    s.set_cue_sound(0, sound(0.5, 100, RATE)).unwrap();
    s.fire_cue(0).unwrap();
    pull(&mut s, 10);
    s.set_cue_loop(0, true).unwrap();
    // Still the non-looping timeline: ends after 100 frames.
    pull(&mut s, 200);
    assert!(!s.cue_state(0).unwrap().playing);
    // Next fire picks up the loop.
    s.fire_cue(0).unwrap();
    pull(&mut s, 500);
    assert!(s.cue_state(0).unwrap().playing);
}

#[test]
fn stop_all_fades_everything() {
    let mut s = session();
    s.set_cue_sound(0, sound(0.5, 100_000, RATE)).unwrap();
    s.set_cue_sound(1, sound(0.5, 100_000, RATE)).unwrap();
    s.fire_cue(0).unwrap();
    s.fire_cue(1).unwrap();
    pull(&mut s, 10);
    s.stop_all(0.01).unwrap(); // 480 frames
    let out = left(&pull(&mut s, 1000));
    assert!(out[0] > 0.9);
    assert!(out[480..].iter().all(|&x| x == 0.0));
    assert!(!s.cue_state(0).unwrap().playing);
}

#[test]
fn gains_apply() {
    let mut s = session();
    s.set_master_gain(0.5).unwrap();
    s.set_cue_sound(0, sound(1.0, 10_000, RATE)).unwrap();
    s.set_cue_gain(0, 0.5).unwrap();
    s.fire_cue(0).unwrap();
    let out = left(&pull(&mut s, 2000));
    // After the 10 ms smoothing ramps settle.
    assert!((out[1500] - 0.25).abs() < 1e-6, "{}", out[1500]);
}

#[test]
fn cues_survive_reopening_the_output() {
    let mut s = session();
    s.set_cue_sound(2, sound(0.5, 1000, RATE)).unwrap();
    s.set_cue_loop(2, true).unwrap();
    s.open_output(None).unwrap();
    s.fire_cue(2).unwrap();
    assert!(left(&pull(&mut s, 2000)).iter().all(|&x| x == 0.5));
}

#[test]
fn reports_device_loss() {
    let mut s = session();
    s.output_stream_mut().unwrap().simulate_device_lost();
    assert_eq!(s.output_state().unwrap().health, StreamHealth::DeviceLost);
    assert_eq!(
        s.output_stream_mut().unwrap().status().health(),
        StreamHealth::DeviceLost
    );
}

#[test]
fn errors() {
    let mut s = Session::new(OfflineBackend::default(), 2);
    s.set_cue_sound(0, sound(0.5, 10, RATE)).unwrap();
    assert!(matches!(s.fire_cue(0), Err(SessionError::NoOutput)));
    assert!(matches!(s.fire_cue(5), Err(SessionError::InvalidCue(5))));
    s.open_output(None).unwrap();
    s.fire_cue(1).unwrap(); // empty cue: no-op
    s.clear_cue(0).unwrap();
    assert!(!s.cue_state(0).unwrap().assigned);
}

#[test]
fn polling_keeps_event_queue_drained() {
    let mut s = session();
    s.set_cue_sound(0, sound(0.5, 10, RATE)).unwrap();
    for _ in 0..10_000 {
        s.fire_cue(0).unwrap();
        pull(&mut s, 20);
        s.poll();
    }
    assert_eq!(s.output_state().unwrap().dropped_events, 0);
}
