//! Sample-accurate behaviour of timeline markers and transport, verified by
//! offline rendering. Every scenario is run at several block sizes.

mod common;

use common::*;
use sbj_engine::{
    Clip, Event, Fade, FadeCurve, LoopCount, Marker, MarkerAction, MarkerId, PlayerId, Timeline,
};

const P: PlayerId = PlayerId(0);

fn concat(parts: &[Vec<f32>]) -> Vec<f32> {
    parts.concat()
}

/// Loads `timeline`, presses play, and returns everything rendered until
/// playback stops.
fn play(timeline: &Timeline, block: usize) -> Vec<f32> {
    let (mut c, mut r) = engine(1);
    c.load(P, timeline.clone()).unwrap();
    c.play(P).unwrap();
    render_until_stopped(&c, &mut r, block, 1_000_000)
}

#[test]
fn plays_clip_then_stops_at_end() {
    let t = Timeline::builder(RATE)
        .clip(Clip::new(ramp_buffer(1000)))
        .build()
        .unwrap();
    for &block in BLOCK_SIZES {
        assert_eq!(play(&t, block), ramp(0..1000), "block {block}");
    }
}

#[test]
fn clip_placement_and_trim() {
    // Frames 100..300 of the source, placed at timeline position 50.
    let t = Timeline::builder(RATE)
        .clip(Clip::new(ramp_buffer(1000)).trimmed(100, 200).at(50))
        .build()
        .unwrap();
    let (mut c, mut r) = engine(1);
    c.load(P, t).unwrap();
    c.play(P).unwrap();
    let out = render(&mut r, 300, 64);
    assert_eq!(&out[..50], &[0.0; 50]);
    assert_eq!(&out[50..250], ramp(100..300).as_slice());
    assert!(!c.is_playing(P), "stops at end of timeline (250)");
}

#[test]
fn jump_marker_moves_playhead_sample_exactly() {
    let t = Timeline::builder(RATE)
        .clip(Clip::new(ramp_buffer(1000)))
        .marker(Marker::new(1, 300, MarkerAction::Jump { to: 700 }))
        .build()
        .unwrap();
    for &block in BLOCK_SIZES {
        assert_eq!(
            play(&t, block),
            concat(&[ramp(0..300), ramp(700..1000)]),
            "block {block}"
        );
    }
}

#[test]
fn loop_repeats_n_times_then_continues() {
    // The headline feature: marker at 600 loops back to 200 twice.
    let t = Timeline::builder(RATE)
        .clip(Clip::new(ramp_buffer(1000)))
        .marker(Marker::new(
            1,
            600,
            MarkerAction::Loop {
                to: 200,
                count: LoopCount::Times(2),
            },
        ))
        .build()
        .unwrap();
    let expected = concat(&[
        ramp(0..600),
        ramp(200..600),
        ramp(200..600),
        ramp(600..1000),
    ]);
    for &block in BLOCK_SIZES {
        assert_eq!(play(&t, block), expected, "block {block}");
    }
}

#[test]
fn infinite_loop_runs_until_operator_intervenes() {
    let t = Timeline::builder(RATE)
        .clip(Clip::new(ramp_buffer(1000)))
        .marker(Marker::new(
            1,
            500,
            MarkerAction::Loop {
                to: 400,
                count: LoopCount::Infinite,
            },
        ))
        .build()
        .unwrap();
    let (mut c, mut r) = engine(1);
    c.load(P, t).unwrap();
    c.play(P).unwrap();
    let out = render(&mut r, 500 + 100 * 50, 64);
    let mut expected = ramp(0..500);
    for _ in 0..50 {
        expected.extend(ramp(400..500));
    }
    assert_eq!(out, expected);
    assert!(c.is_playing(P));

    c.stop(P).unwrap();
    render(&mut r, 64, 64);
    assert!(!c.is_playing(P));
    assert_eq!(c.position(P), 0);
}

#[test]
fn nested_loops_rearm_like_musical_repeats() {
    // Inner loop: 300 -> 200 once. Outer loop: 500 -> 100 once.
    // Inner counter re-arms when passed, so it repeats on the outer pass.
    let t = Timeline::builder(RATE)
        .clip(Clip::new(ramp_buffer(600)))
        .marker(Marker::new(
            1,
            300,
            MarkerAction::Loop {
                to: 200,
                count: LoopCount::Times(1),
            },
        ))
        .marker(Marker::new(
            2,
            500,
            MarkerAction::Loop {
                to: 100,
                count: LoopCount::Times(1),
            },
        ))
        .build()
        .unwrap();
    let expected = concat(&[ramp(0..300), ramp(200..500), ramp(100..300), ramp(200..600)]);
    for &block in BLOCK_SIZES {
        assert_eq!(play(&t, block), expected, "block {block}");
    }
}

#[test]
fn markers_at_jump_landing_do_not_fire() {
    // The loop lands on a Stop marker; the stop must be skipped, otherwise a
    // jump to another marker could create surprising chains or loops.
    let t = Timeline::builder(RATE)
        .clip(Clip::new(ramp_buffer(1000)))
        .marker(Marker::new(1, 100, MarkerAction::Stop))
        .marker(Marker::new(
            2,
            400,
            MarkerAction::Loop {
                to: 100,
                count: LoopCount::Times(1),
            },
        ))
        .marker(Marker::new(3, 600, MarkerAction::Stop))
        .build()
        .unwrap();
    let (mut c, mut r) = engine(1);
    c.load(P, t).unwrap();
    c.seek(P, 101).unwrap();
    c.play(P).unwrap();
    let out = render_until_stopped(&c, &mut r, 64, 10_000);
    assert_eq!(out, concat(&[ramp(101..400), ramp(100..600)]));
}

#[test]
fn self_targeting_jump_cannot_hang() {
    let t = Timeline::builder(RATE)
        .clip(Clip::new(ramp_buffer(300)))
        .marker(Marker::new(1, 100, MarkerAction::Jump { to: 100 }))
        .build()
        .unwrap();
    assert_eq!(play(&t, 64), ramp(0..300));
}

#[test]
fn pause_marker_halts_and_resume_continues_past_it() {
    let t = Timeline::builder(RATE)
        .clip(Clip::new(ramp_buffer(1000)))
        .marker(Marker::new(7, 250, MarkerAction::Pause))
        .build()
        .unwrap();
    for &block in BLOCK_SIZES {
        let (mut c, mut r) = engine(1);
        c.load(P, t.clone()).unwrap();
        c.play(P).unwrap();
        assert_eq!(
            render_until_stopped(&c, &mut r, block, 10_000),
            ramp(0..250)
        );
        assert_eq!(c.position(P), 250);

        c.play(P).unwrap();
        assert_eq!(
            render_until_stopped(&c, &mut r, block, 10_000),
            ramp(250..1000),
            "block {block}"
        );
    }
}

#[test]
fn stop_marker_rewinds() {
    let t = Timeline::builder(RATE)
        .clip(Clip::new(ramp_buffer(1000)))
        .marker(Marker::new(1, 400, MarkerAction::Stop))
        .build()
        .unwrap();
    let (mut c, mut r) = engine(1);
    c.load(P, t).unwrap();
    c.play(P).unwrap();
    assert_eq!(render_until_stopped(&c, &mut r, 64, 10_000), ramp(0..400));
    assert_eq!(c.position(P), 0);
}

#[test]
fn fade_out_marker_reaches_silence_and_stops_exactly() {
    let t = Timeline::builder(RATE)
        .clip(Clip::new(ramp_buffer(1000)))
        .marker(Marker::new(1, 200, MarkerAction::FadeOut { length: 100 }))
        .build()
        .unwrap();
    for &block in BLOCK_SIZES {
        let out = play(&t, block);
        assert_eq!(out.len(), 300, "block {block}");
        assert_eq!(&out[..200], ramp(0..200).as_slice());
        // Gain goes 100/100, 99/100, ... 1/100 over frames 200..300.
        for (k, &s) in out[200..].iter().enumerate() {
            let expected = (201 + k) as f32 * (100 - k) as f32 / 100.0;
            assert!(
                (s - expected).abs() < 1e-3,
                "frame {} block {block}",
                200 + k
            );
        }
    }
}

#[test]
fn several_markers_at_one_position_fire_in_order() {
    // FadeOut starts, then Jump moves the playhead: both take effect.
    let t = Timeline::builder(RATE)
        .clip(Clip::new(ramp_buffer(1000)))
        .marker(Marker::new(1, 100, MarkerAction::FadeOut { length: 50 }))
        .marker(Marker::new(2, 100, MarkerAction::Jump { to: 800 }))
        .build()
        .unwrap();
    let (mut c, mut r) = engine(1);
    c.load(P, t).unwrap();
    c.play(P).unwrap();
    let out = render_until_stopped(&c, &mut r, 64, 10_000);
    assert_eq!(out.len(), 150);
    assert_eq!(out[100], 801.0);
    assert!(!c.is_playing(P));

    let fired: Vec<_> = events(&mut c)
        .into_iter()
        .filter_map(|e| match e {
            Event::MarkerReached { marker, .. } => Some(marker),
            _ => None,
        })
        .collect();
    assert_eq!(fired, vec![MarkerId(1), MarkerId(2)]);
}

#[test]
fn transport_events() {
    let t = Timeline::builder(RATE)
        .clip(Clip::new(ramp_buffer(100)))
        .marker(Marker::new(9, 50, MarkerAction::Jump { to: 60 }))
        .build()
        .unwrap();
    let (mut c, mut r) = engine(1);
    c.load(P, t).unwrap();
    c.play(P).unwrap();
    render(&mut r, 200, 64);
    assert_eq!(
        events(&mut c),
        vec![
            Event::TimelineLoaded { player: P },
            Event::Started {
                player: P,
                position: 0
            },
            Event::MarkerReached {
                player: P,
                marker: MarkerId(9),
                position: 50
            },
            Event::ReachedEnd { player: P },
        ]
    );
}

#[test]
fn seek_resets_loop_counters() {
    let t = Timeline::builder(RATE)
        .clip(Clip::new(ramp_buffer(400)))
        .marker(Marker::new(
            1,
            300,
            MarkerAction::Loop {
                to: 200,
                count: LoopCount::Times(1),
            },
        ))
        .build()
        .unwrap();
    let (mut c, mut r) = engine(1);
    c.load(P, t).unwrap();
    c.seek(P, 250).unwrap();
    c.play(P).unwrap();
    // Play 250..300, loop back once, play 200..250: the loop counter is now
    // spent. Seeking must re-arm it.
    render(&mut r, 100, 10);
    c.seek(P, 250).unwrap();
    let out = render_until_stopped(&c, &mut r, 10, 10_000);
    assert_eq!(out, concat(&[ramp(250..300), ramp(200..400)]));
}

#[test]
fn clip_fades_and_gain() {
    let t = Timeline::builder(RATE)
        .clip(
            Clip::new(std::sync::Arc::new(
                sbj_engine::AudioBuffer::from_interleaved(RATE, 1, vec![1.0; 100]).unwrap(),
            ))
            .with_gain(0.5)
            .with_fade_in(Fade::new(10, FadeCurve::Linear))
            .with_fade_out(Fade::new(20, FadeCurve::Linear)),
        )
        .build()
        .unwrap();
    let (mut c, mut r) = engine(1);
    c.load(P, t).unwrap();
    c.play(P).unwrap();
    let out = render(&mut r, 100, 33);
    assert_eq!(out[0], 0.0);
    assert!((out[5] - 0.25).abs() < 1e-6);
    assert_eq!(out[50], 0.5);
    assert!((out[90] - 0.25).abs() < 1e-6);
    assert!(out[99] > 0.0 && out[99] < 0.05);
}

#[test]
fn mono_clips_feed_all_channels_and_stereo_maps_by_channel() {
    let stereo = std::sync::Arc::new(
        sbj_engine::AudioBuffer::from_interleaved(RATE, 2, vec![1.0, 2.0, 1.0, 2.0]).unwrap(),
    );
    let t = Timeline::builder(RATE)
        .clip(Clip::new(ramp_buffer(2)))
        .clip(Clip::new(stereo))
        .build()
        .unwrap();
    let (mut c, mut r) = engine(4);
    c.load(P, t).unwrap();
    c.play(P).unwrap();
    let out = render(&mut r, 2, 2);
    assert_eq!(out, vec![2.0, 3.0, 1.0, 1.0, 3.0, 4.0, 2.0, 2.0]);
}

#[test]
fn master_gain_is_smoothed() {
    let mut config = sbj_engine::EngineConfig::new(RATE, 1);
    config.declick_frames = 0;
    config.gain_ramp_frames = 10;
    let (mut c, mut r) = sbj_engine::Engine::create(config).unwrap();
    let t = Timeline::builder(RATE)
        .clip(Clip::new(std::sync::Arc::new(
            sbj_engine::AudioBuffer::from_interleaved(RATE, 1, vec![1.0; 100]).unwrap(),
        )))
        .build()
        .unwrap();
    c.load(P, t).unwrap();
    c.set_master_gain(0.0).unwrap();
    c.play(P).unwrap();
    let out = render(&mut r, 20, 20);
    assert!((out[0] - 0.9).abs() < 1e-6);
    assert!(out.windows(2).take(9).all(|w| w[1] < w[0]));
    assert!(out[10..].iter().all(|&s| s == 0.0));
}

#[test]
fn replacing_timeline_returns_old_one_for_freeing_off_the_audio_thread() {
    let t = Timeline::builder(RATE)
        .clip(Clip::new(ramp_buffer(100)))
        .build()
        .unwrap();
    let (mut c, mut r) = engine(1);
    c.load(P, t.clone()).unwrap();
    render(&mut r, 1, 1);
    c.load(P, t).unwrap();
    render(&mut r, 1, 1);
    assert_eq!(c.collect_garbage(), 1);
    assert_eq!(c.leaked_garbage(), 0);
}

#[test]
fn rejects_timeline_with_wrong_sample_rate() {
    let t = Timeline::builder(44_100).build().unwrap();
    let (mut c, _r) = engine(1);
    assert!(matches!(
        c.load(P, t),
        Err(sbj_engine::EngineError::SampleRateMismatch { .. })
    ));
}

// ---------------------------------------------------------------------
// Multiple players
// ---------------------------------------------------------------------

fn constant(value: f32, frames: usize) -> Timeline {
    Timeline::builder(RATE)
        .clip(Clip::new(std::sync::Arc::new(
            sbj_engine::AudioBuffer::from_interleaved(RATE, 1, vec![value; frames]).unwrap(),
        )))
        .build()
        .unwrap()
}

#[test]
fn players_mix_and_run_independently() {
    let (a, b) = (PlayerId(0), PlayerId(5));
    let (mut c, mut r) = engine(1);
    c.load(a, constant(1.0, 100)).unwrap();
    c.load(b, constant(10.0, 50)).unwrap();
    c.play(a).unwrap();
    c.play(b).unwrap();
    let out = render(&mut r, 120, 32);
    assert!(out[..50].iter().all(|&s| s == 11.0));
    assert!(out[50..100].iter().all(|&s| s == 1.0));
    assert!(out[100..].iter().all(|&s| s == 0.0));
    assert!(!c.is_playing(a) && !c.is_playing(b));
}

#[test]
fn restart_plays_from_the_top_even_while_playing() {
    let (mut c, mut r) = engine(1);
    c.load(
        P,
        Timeline::builder(RATE)
            .clip(Clip::new(ramp_buffer(100)))
            .build()
            .unwrap(),
    )
    .unwrap();
    c.play(P).unwrap();
    render(&mut r, 40, 40);
    c.restart(P).unwrap();
    assert_eq!(render(&mut r, 10, 10), ramp(0..10));
}

#[test]
fn player_gain_and_stop_all() {
    let (a, b) = (PlayerId(1), PlayerId(2));
    let (mut c, mut r) = engine(1);
    c.load(a, constant(1.0, 1000)).unwrap();
    c.load(b, constant(1.0, 1000)).unwrap();
    c.set_player_gain(b, 0.5).unwrap();
    c.play(a).unwrap();
    c.play(b).unwrap();
    assert!(render(&mut r, 10, 10).iter().all(|&s| s == 1.5));

    // Panic: fade everything out over 100 frames.
    c.stop_all(100).unwrap();
    let out = render(&mut r, 200, 64);
    assert!((out[0] - 1.5).abs() < 1e-6);
    assert!(out[..100].windows(2).all(|w| w[1] <= w[0]));
    assert!(out[100..].iter().all(|&s| s == 0.0));
    assert!(!c.is_playing(a) && !c.is_playing(b));
    assert_eq!(c.position(a), 0);
}

#[test]
fn unload_stops_and_returns_timeline_for_freeing() {
    let (mut c, mut r) = engine(1);
    c.load(P, constant(1.0, 100)).unwrap();
    c.play(P).unwrap();
    render(&mut r, 10, 10);
    c.unload(P).unwrap();
    assert!(render(&mut r, 10, 10).iter().all(|&s| s == 0.0));
    assert!(!c.is_playing(P));
    assert_eq!(c.collect_garbage(), 1);
}

#[test]
fn rejects_unknown_player() {
    let (mut c, _r) = engine(1);
    let bad = PlayerId(c.config().players);
    assert_eq!(
        c.play(bad),
        Err(sbj_engine::EngineError::InvalidPlayer(bad))
    );
    assert!(!c.is_playing(bad));
}

#[test]
fn device_blocks_larger_than_the_mix_block_are_split() {
    let mut config = sbj_engine::EngineConfig::new(RATE, 2);
    config.declick_frames = 0;
    config.max_block_frames = 16;
    config.gain_ramp_frames = 0;
    let (mut c, mut r) = sbj_engine::Engine::create(config).unwrap();
    c.load(
        P,
        Timeline::builder(RATE)
            .clip(Clip::new(ramp_buffer(100)))
            .build()
            .unwrap(),
    )
    .unwrap();
    c.play(P).unwrap();
    let out = render(&mut r, 100, 100);
    let left: Vec<f32> = out.chunks(2).map(|f| f[0]).collect();
    assert_eq!(left, ramp(0..100));
}

#[test]
fn gain_set_while_stopped_applies_instantly_but_ramps_while_playing() {
    let mut config = sbj_engine::EngineConfig::new(RATE, 1);
    config.declick_frames = 0;
    config.gain_ramp_frames = 100;
    let (mut c, mut r) = sbj_engine::Engine::create(config).unwrap();
    c.load(P, constant(1.0, 10_000)).unwrap();
    c.set_player_gain(P, 0.5).unwrap();
    c.play(P).unwrap();
    let out = render(&mut r, 10, 10);
    assert!(out.iter().all(|&s| s == 0.5), "no ramp from the old gain");

    c.set_player_gain(P, 0.0).unwrap();
    let out = render(&mut r, 200, 50);
    assert!(out[0] > 0.49 && out[0] < 0.5, "smoothed while audible");
    assert!(out[100..].iter().all(|&s| s == 0.0));
}

// ---------------------------------------------------------------------
// Declick crossfades
// ---------------------------------------------------------------------

const DECLICK: u32 = 10;

fn declick_engine() -> (sbj_engine::Controller, sbj_engine::Renderer) {
    let mut config = sbj_engine::EngineConfig::new(RATE, 1);
    config.gain_ramp_frames = 0;
    config.declick_frames = DECLICK;
    sbj_engine::Engine::create(config).unwrap()
}

/// Expected blend of new frame values `new` fading in over old `old`.
fn blend(new: &[f32], old: &[f32]) -> Vec<f32> {
    new.iter()
        .zip(old)
        .enumerate()
        .map(|(f, (n, o))| {
            let k = f as f32 / DECLICK as f32;
            n * k + o * (1.0 - k)
        })
        .collect()
}

fn close(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3)
}

#[test]
fn jumps_crossfade_instead_of_clicking() {
    let t = Timeline::builder(RATE)
        .clip(Clip::new(ramp_buffer(1000)))
        .marker(Marker::new(1, 300, MarkerAction::Jump { to: 700 }))
        .build()
        .unwrap();
    for &block in BLOCK_SIZES {
        let (mut c, mut r) = declick_engine();
        c.load(P, t.clone()).unwrap();
        c.play(P).unwrap();
        let out = render(&mut r, 400, block);
        assert_eq!(&out[..300], ramp(0..300).as_slice(), "block {block}");
        // The old audio (300..) fades out as the new (700..) fades in.
        let expected = blend(&ramp(700..710), &ramp(300..310));
        assert!(
            close(&out[300..310], &expected),
            "block {block}: {:?}",
            &out[300..310]
        );
        assert_eq!(&out[310..400], ramp(710..800).as_slice(), "block {block}");
    }
}

#[test]
fn retrigger_while_playing_crossfades() {
    let (mut c, mut r) = declick_engine();
    c.load(
        P,
        Timeline::builder(RATE)
            .clip(Clip::new(ramp_buffer(1000)))
            .build()
            .unwrap(),
    )
    .unwrap();
    c.play(P).unwrap();
    render(&mut r, 500, 64);
    c.restart(P).unwrap();
    let out = render(&mut r, 20, 7);
    assert!(close(&out[..10], &blend(&ramp(0..10), &ramp(500..510))));
    assert_eq!(&out[10..], ramp(10..20).as_slice());
}

#[test]
fn operator_stop_fades_briefly_then_stops() {
    let (mut c, mut r) = declick_engine();
    c.load(P, constant(1.0, 1000)).unwrap();
    c.play(P).unwrap();
    render(&mut r, 100, 64);
    c.stop(P).unwrap();
    let out = render(&mut r, 50, 50);
    assert!((out[0] - 1.0).abs() < 1e-6);
    assert!(out[..DECLICK as usize].windows(2).all(|w| w[1] < w[0]));
    assert!(out[DECLICK as usize..].iter().all(|&s| s == 0.0));
    assert!(!c.is_playing(P));
    assert_eq!(c.position(P), 0);
}

#[test]
fn stopping_a_paused_player_is_immediate() {
    let (mut c, mut r) = declick_engine();
    c.load(P, constant(1.0, 1000)).unwrap();
    c.play(P).unwrap();
    render(&mut r, 100, 64);
    c.pause(P).unwrap();
    c.stop(P).unwrap();
    render(&mut r, 1, 1);
    assert_eq!(c.position(P), 0);
}

// ---------------------------------------------------------------------
// Vamps (until-released loops), routing, ramped gain
// ---------------------------------------------------------------------

fn vamp_timeline() -> Timeline {
    // Intro 0..200, vamp 200..400 (loops until released), outro 400..600.
    Timeline::builder(RATE)
        .clip(Clip::new(ramp_buffer(600)))
        .marker(Marker::new(
            1,
            400,
            MarkerAction::Loop {
                to: 200,
                count: LoopCount::UntilReleased,
            },
        ))
        .build()
        .unwrap()
}

#[test]
fn vamp_loops_until_released_then_exits_at_the_loop_point() {
    for &block in BLOCK_SIZES {
        let (mut c, mut r) = engine(1);
        c.load(P, vamp_timeline()).unwrap();
        c.play(P).unwrap();
        // Intro plus three passes of the vamp.
        let out = render(&mut r, 200 + 3 * 200 + 50, block);
        assert_eq!(&out[..200], ramp(0..200).as_slice());
        for pass in 0..3 {
            let at = 200 + pass * 200;
            assert_eq!(
                &out[at..at + 200],
                ramp(200..400).as_slice(),
                "pass {pass} block {block}"
            );
        }
        assert!(c.is_holding(P));

        // Released mid-vamp: finish this pass, then play the outro.
        c.release(P).unwrap();
        let rest = render_until_stopped(&c, &mut r, block, 10_000);
        assert_eq!(
            rest,
            concat(&[ramp(250..400), ramp(400..600)]),
            "block {block}"
        );
        assert!(!c.is_holding(P));
    }
}

#[test]
fn release_applies_once_and_restart_rearms() {
    // Two vamps in a row: one release only gets past the first.
    let t = Timeline::builder(RATE)
        .clip(Clip::new(ramp_buffer(600)))
        .marker(Marker::new(
            1,
            200,
            MarkerAction::Loop {
                to: 100,
                count: LoopCount::UntilReleased,
            },
        ))
        .marker(Marker::new(
            2,
            400,
            MarkerAction::Loop {
                to: 300,
                count: LoopCount::UntilReleased,
            },
        ))
        .build()
        .unwrap();
    let (mut c, mut r) = engine(1);
    c.load(P, t).unwrap();
    c.play(P).unwrap();
    c.release(P).unwrap(); // before even reaching the first vamp
    let out = render(&mut r, 200 + 200 + 300, 64);
    assert_eq!(
        &out[..400],
        ramp(0..400).as_slice(),
        "passes the first vamp"
    );
    assert_eq!(
        &out[400..500],
        ramp(300..400).as_slice(),
        "holds at the second"
    );
    assert!(c.is_holding(P));

    c.restart(P).unwrap();
    let out = render(&mut r, 300, 64);
    assert_eq!(
        &out[200..300],
        ramp(100..200).as_slice(),
        "restart re-arms the hold"
    );
}

#[test]
fn routing_sends_to_chosen_channels() {
    let (mut c, mut r) = engine(4);
    c.load(P, constant(0.5, 100)).unwrap();
    c.set_route(P, 2, 2).unwrap(); // channels 3-4
    c.play(P).unwrap();
    let out = render(&mut r, 2, 2);
    assert_eq!(out, vec![0.0, 0.0, 0.5, 0.5, 0.0, 0.0, 0.5, 0.5]);

    // A route past the device's channels is clamped, not a crash.
    c.set_route(P, 3, 2).unwrap();
    let out = render(&mut r, 1, 1);
    assert_eq!(out, vec![0.0, 0.0, 0.0, 0.5]);
    c.set_route(P, 9, 2).unwrap();
    assert_eq!(
        render(&mut r, 1, 1),
        vec![0.5; 4],
        "out of range: all channels"
    );
    c.set_route(P, 0, 0).unwrap();
    assert_eq!(render(&mut r, 1, 1), vec![0.5; 4]);
}

#[test]
fn ramped_gain_takes_the_requested_time() {
    let (mut c, mut r) = engine(1);
    c.load(P, constant(1.0, 10_000)).unwrap();
    c.play(P).unwrap();
    render(&mut r, 10, 10);
    c.set_player_gain_over(P, 0.0, 1000).unwrap();
    let out = render(&mut r, 1100, 128);
    assert!((out[499] - 0.5).abs() < 0.01, "{}", out[499]);
    assert!(out[1000..].iter().all(|&s| s == 0.0));
}

// ---------------------------------------------------------------------
// Markers that act on other players, and level markers
// ---------------------------------------------------------------------

/// `constant(value, frames)` with extra markers.
fn constant_with(value: f32, frames: usize, markers: &[Marker]) -> Timeline {
    let mut builder = Timeline::builder(RATE).clip(Clip::new(std::sync::Arc::new(
        sbj_engine::AudioBuffer::from_interleaved(RATE, 1, vec![value; frames]).unwrap(),
    )));
    for m in markers {
        builder = builder.marker(m.clone());
    }
    builder.build().unwrap()
}

fn start(id: u64, at: u64, player: PlayerId) -> Marker {
    Marker::new(id, at, MarkerAction::Start { player, fade_in: 0 })
}

fn runs(out: &[f32], expected: &[(usize, f32)]) -> bool {
    let mut from = 0;
    for &(to, value) in expected {
        if out[from..to].iter().any(|&s| (s - value).abs() > 1e-5) {
            eprintln!("expected {value} in {from}..{to}: {:?}", &out[from..to]);
            return false;
        }
        from = to;
    }
    true
}

#[test]
fn start_marker_fires_another_player_on_the_exact_sample() {
    // The target is rendered before (2) and after (9) the source (5) in
    // the renderer's order; both must start exactly at frame 37.
    for target in [PlayerId(2), PlayerId(9)] {
        for &block in BLOCK_SIZES {
            let source = PlayerId(5);
            let (mut c, mut r) = engine(1);
            c.load(source, constant_with(1.0, 100, &[start(1, 37, target)]))
                .unwrap();
            c.load(target, constant(10.0, 20)).unwrap();
            c.play(source).unwrap();
            let out = render(&mut r, 120, block);
            assert!(
                runs(&out, &[(37, 1.0), (57, 11.0), (100, 1.0), (120, 0.0)]),
                "target {target:?}, block {block}"
            );
            let events = events(&mut c);
            assert!(events.contains(&Event::Triggered {
                player: target,
                by: source
            }));
        }
    }
}

#[test]
fn start_markers_chain_sample_exactly() {
    // A starts B at 10; B starts C 5 frames into B (frame 15).
    for &block in BLOCK_SIZES {
        let (a, b, cc) = (PlayerId(4), PlayerId(6), PlayerId(1));
        let (mut c, mut r) = engine(1);
        c.load(a, constant_with(1.0, 100, &[start(1, 10, b)]))
            .unwrap();
        c.load(b, constant_with(10.0, 50, &[start(1, 5, cc)]))
            .unwrap();
        c.load(cc, constant(100.0, 20)).unwrap();
        c.play(a).unwrap();
        let out = render(&mut r, 100, block);
        assert!(
            runs(
                &out,
                &[(10, 1.0), (15, 11.0), (35, 111.0), (60, 11.0), (100, 1.0)]
            ),
            "block {block}"
        );
    }
}

#[test]
fn start_marker_restarts_a_playing_target_and_fades_in() {
    for &block in BLOCK_SIZES {
        let (a, b) = (PlayerId(0), PlayerId(1));
        let (mut c, mut r) = engine(1);
        c.load(
            a,
            constant_with(
                0.0,
                100,
                &[Marker::new(
                    1,
                    40,
                    MarkerAction::Start {
                        player: b,
                        fade_in: 10,
                    },
                )],
            ),
        )
        .unwrap();
        c.load(
            b,
            Timeline::builder(RATE)
                .clip(Clip::new(ramp_buffer(200)))
                .build()
                .unwrap(),
        )
        .unwrap();
        c.play(b).unwrap();
        c.play(a).unwrap();
        let out = render(&mut r, 60, block);
        assert_eq!(&out[..40], &ramp(0..40)[..], "block {block}");
        // Restarted at frame 40, fading in over 10 frames.
        for i in 0..10 {
            let expected = (i + 1) as f32 * (i + 1) as f32 / 10.0;
            assert!((out[40 + i] - expected).abs() < 1e-4, "block {block}, {i}");
        }
        assert_eq!(&out[50..60], &ramp(10..20)[..], "block {block}");
    }
}

#[test]
fn start_marker_releases_a_vamping_target() {
    let (a, b) = (PlayerId(0), PlayerId(1));
    let (mut c, mut r) = engine(1);
    c.load(a, constant_with(0.0, 1_000, &[start(1, 250, b)]))
        .unwrap();
    c.load(b, vamp_timeline()).unwrap();
    c.play(b).unwrap();
    render(&mut r, 100, 64);
    c.play(a).unwrap();
    render(&mut r, 50, 64);
    assert!(c.is_holding(b));
    render(&mut r, 300, 64);
    assert!(!c.is_holding(b), "released, not restarted");
    assert!(c.position(b) > 400, "played on past the vamp");
}

#[test]
fn stop_player_marker_stops_the_target_exactly() {
    for &block in BLOCK_SIZES {
        let (a, b) = (PlayerId(3), PlayerId(2));
        let stop = |fade| Marker::new(1, 40, MarkerAction::StopPlayer { player: b, fade });
        // Immediately (declick is off in this engine).
        let (mut c, mut r) = engine(1);
        c.load(a, constant_with(1.0, 100, &[stop(0)])).unwrap();
        c.load(b, constant(10.0, 100)).unwrap();
        c.play(a).unwrap();
        c.play(b).unwrap();
        let out = render(&mut r, 100, block);
        assert!(runs(&out, &[(40, 11.0), (100, 1.0)]), "block {block}");
        assert!(!c.is_playing(b));

        // Over 10 frames.
        let (mut c, mut r) = engine(1);
        c.load(a, constant_with(1.0, 100, &[stop(10)])).unwrap();
        c.load(b, constant(10.0, 100)).unwrap();
        c.play(a).unwrap();
        c.play(b).unwrap();
        let out = render(&mut r, 100, block);
        assert!(runs(&out, &[(40, 11.0)]), "block {block}");
        for i in 0..10 {
            let expected = 1.0 + 10.0 * (10 - i) as f32 / 10.0;
            assert!((out[40 + i] - expected).abs() < 1e-4, "block {block}, {i}");
        }
        assert!(runs(&out[50..], &[(50, 1.0)]), "block {block}");
    }
}

#[test]
fn level_markers_ramp_this_player_and_reset_on_restart() {
    let markers = [
        Marker::new(
            1,
            20,
            MarkerAction::Level {
                gain: 0.5,
                length: 0,
            },
        ),
        Marker::new(
            2,
            60,
            MarkerAction::Level {
                gain: 1.0,
                length: 10,
            },
        ),
    ];
    for &block in BLOCK_SIZES {
        let (mut c, mut r) = engine(1);
        c.load(P, constant_with(1.0, 100, &markers)).unwrap();
        c.play(P).unwrap();
        let out = render(&mut r, 100, block);
        assert!(runs(&out, &[(20, 1.0), (60, 0.5)]), "block {block}");
        for i in 0..10 {
            let expected = 0.5 + 0.05 * (i + 1) as f32;
            assert!((out[60 + i] - expected).abs() < 1e-5, "block {block}, {i}");
        }
        assert!(runs(&out[70..], &[(30, 1.0)]), "block {block}");

        // Restarting after the first marker starts at full level again.
        c.play(P).unwrap();
        render(&mut r, 30, block);
        c.restart(P).unwrap();
        assert!(render(&mut r, 10, block).iter().all(|&s| s == 1.0));
    }
}

#[test]
fn notify_markers_only_report() {
    let (mut c, mut r) = engine(1);
    c.load(
        P,
        constant_with(1.0, 50, &[Marker::new(7, 10, MarkerAction::Notify)]),
    )
    .unwrap();
    c.play(P).unwrap();
    assert!(render(&mut r, 50, 16).iter().all(|&s| s == 1.0));
    assert!(events(&mut c).contains(&Event::MarkerReached {
        player: P,
        marker: MarkerId(7),
        position: 10
    }));
}

#[test]
fn start_marker_ignores_missing_and_empty_players() {
    let (mut c, mut r) = engine(1);
    let markers = [start(1, 5, PlayerId(1)), start(2, 6, PlayerId(10_000))];
    c.load(P, constant_with(1.0, 20, &markers)).unwrap();
    c.play(P).unwrap();
    assert!(render(&mut r, 20, 7).iter().all(|&s| s == 1.0));
    assert!(
        !events(&mut c)
            .iter()
            .any(|e| matches!(e, Event::Triggered { .. }))
    );
}
