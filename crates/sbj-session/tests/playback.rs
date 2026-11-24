//! Firing behaviour: vamps, retrigger modes, exclusive groups, ducking,
//! channel routing.

mod common;

use common::*;
use sbj_backend::offline::{OfflineBackend, OfflineDevice};
use sbj_session::{
    ActionDoc, ActionKind, CueDoc, MarkerDoc, Retrigger, RouteDoc, Session, TargetDoc,
};

fn cue(slot: u32, file: &str) -> CueDoc {
    CueDoc::simple(slot, file, file)
}

#[test]
fn pressing_a_vamping_pad_lets_it_continue() {
    let backend = two_devices();
    let mut s = session(&backend);
    s.add_sound("r".into(), ramp(48_000));
    let mut c = cue(0, "r");
    // Vamp 0.25-0.5 s until released.
    c.markers.push(MarkerDoc {
        id: 1,
        name: "vamp".into(),
        time: 0.5,
        action: ActionDoc {
            to: Some(TargetDoc {
                time: Some(0.25),
                marker: None,
            }),
            hold: true,
            ..ActionDoc::new(ActionKind::Loop)
        },
    });
    s.set_cue(c).unwrap();
    s.fire_cue(0).unwrap();
    pull(&mut s, 0, 24_000 + 30_000); // well into the vamp
    let st = s.cue_state(0).unwrap();
    assert!(st.playing && st.holding);

    s.fire_cue(0).unwrap(); // "go"
    let rest = pull(&mut s, 0, 48_000);
    assert!(!s.cue_state(0).unwrap().playing, "played out to the end");
    let audible = rest.iter().filter(|x| x.abs() > 0.0).count();
    // Finished the current vamp pass (6000 left) then the outro (24000).
    assert_eq!(audible, 6_000 + 24_000);
}

#[test]
fn retrigger_modes() {
    let backend = two_devices();
    let mut s = session(&backend);
    s.add_sound("a".into(), constant(0.5, 100_000));
    let mut stop = cue(0, "a");
    stop.retrigger = Retrigger::Stop;
    let mut fade = cue(1, "a");
    fade.retrigger = Retrigger::Fade;
    s.set_cue(stop).unwrap();
    s.set_cue(fade).unwrap();

    s.fire_cue(0).unwrap();
    pull(&mut s, 0, 100);
    s.fire_cue(0).unwrap();
    pull(&mut s, 0, 100);
    assert!(!s.cue_state(0).unwrap().playing, "second press stops");

    s.fire_cue(1).unwrap();
    pull(&mut s, 0, 100);
    s.fire_cue(1).unwrap();
    let out = pull(&mut s, 0, 30_000);
    assert!(out[0] > 0.45 && out[12_000] < 0.4 && out[24_000..].iter().all(|&x| x == 0.0));
    assert!(!s.cue_state(1).unwrap().playing, "second press fades out");
}

#[test]
fn exclusive_group_hands_over() {
    let backend = two_devices();
    let mut s = session(&backend);
    s.add_sound("a".into(), constant(0.25, 200_000));
    s.add_sound("b".into(), constant(0.5, 200_000));
    let mut first = cue(0, "a");
    first.group = Some("Music".into());
    let mut second = cue(1, "b");
    second.group = Some("music ".into()); // case and spaces don't matter
    let mut other = cue(2, "a");
    other.group = Some("Horns".into());
    s.set_cue(first).unwrap();
    s.set_cue(second).unwrap();
    s.set_cue(other).unwrap();

    s.fire_cue(0).unwrap();
    s.fire_cue(2).unwrap();
    pull(&mut s, 0, 1000);
    s.fire_cue(1).unwrap();
    // Group fade is 1 s: after it, only "b" and the other group remain.
    pull(&mut s, 0, 49_000);
    assert!(!s.cue_state(0).unwrap().playing, "faded out by its group");
    assert!(s.cue_state(1).unwrap().playing);
    assert!(s.cue_state(2).unwrap().playing, "different group untouched");
    assert!(pull(&mut s, 0, 10).iter().all(|&x| (x - 0.75).abs() < 1e-6));
}

#[test]
fn ducking_lowers_others_and_recovers() {
    let backend = two_devices();
    let mut s = session(&backend);
    s.add_sound("music".into(), constant(0.5, 500_000));
    s.add_sound("voice".into(), constant(0.1, 48_000)); // 1 s
    s.set_cue(cue(0, "music")).unwrap();
    let mut voice = cue(1, "voice");
    voice.duck = Some(-20.0); // music to 1/10
    s.set_cue(voice).unwrap();

    s.fire_cue(0).unwrap();
    pull(&mut s, 0, 100);
    s.fire_cue(1).unwrap();
    let out = pull(&mut s, 0, 24_000);
    s.poll(); // the app polls every ~33 ms while things play
    // After the 150 ms attack: music 0.05 + voice 0.1.
    assert!((out[20_000] - 0.15).abs() < 1e-3, "{}", out[20_000]);

    // Voice ends after 1 s; polling notices and music comes back up.
    pull(&mut s, 0, 30_000);
    s.poll();
    pull(&mut s, 0, 48_000); // 0.8 s release
    let out = pull(&mut s, 0, 10);
    assert!((out[0] - 0.5).abs() < 1e-3, "{}", out[0]);
}

#[test]
fn routing_to_channel_pairs() {
    let backend = OfflineBackend::with_devices(vec![{
        let mut d = OfflineDevice::new("eight", 48_000, 8);
        d.is_default = true;
        d
    }]);
    let mut s = Session::new(backend);
    s.set_declick(0.0);
    s.open_outputs();
    s.add_sound("a".into(), constant(0.5, 1000));
    let mut c = cue(0, "a");
    c.route = Some(RouteDoc { first: 3, count: 2 });
    s.set_cue(c).unwrap();
    assert!(s.problems().is_empty());
    s.fire_cue(0).unwrap();
    let frame: Vec<f32> = s.output_stream_mut(0).unwrap().render(1)[..8].to_vec();
    assert_eq!(frame, vec![0.0, 0.0, 0.5, 0.5, 0.0, 0.0, 0.0, 0.0]);

    // Channels the device doesn't have: reported, plays on all.
    let mut c = cue(0, "a");
    c.route = Some(RouteDoc { first: 7, count: 4 });
    s.set_cue(c).unwrap();
    assert!(s.problems()[0].message.contains("channels 7-10"));
}

// ---------------------------------------------------------------------
// Playlists and fades
// ---------------------------------------------------------------------

use sbj_session::PlaylistDoc;

/// Pad 0 is a playlist of three constant-value songs a/b/c (0.1/0.2/0.3,
/// 4800 frames = 0.1 s each) with a 10 ms crossfade.
fn playlist_session(auto_advance: bool, shuffle: bool) -> Session<OfflineBackend> {
    let backend = two_devices();
    let mut s = session(&backend);
    for (name, value) in [("a", 0.1), ("b", 0.2), ("c", 0.3)] {
        s.add_sound(name.into(), constant(value, 4_800));
    }
    let mut c = CueDoc::simple(0, "Timeout", "unused");
    c.clips.clear();
    c.playlist = Some(PlaylistDoc {
        items: vec!["a".into(), "b".into(), "c".into()],
        shuffle,
        auto_advance,
        crossfade: 0.01,
    });
    s.set_cue(c).unwrap();
    assert!(s.problems().is_empty(), "{:?}", s.problems());
    s
}

#[test]
fn playlist_plays_one_item_per_press_and_crossfades_on_skip() {
    let mut s = playlist_session(false, false);
    assert!(
        s.preflight()
            .checks
            .iter()
            .all(|c| !c.message.contains("no audio")),
        "a playlist pad has audio"
    );
    s.fire_cue(0).unwrap();
    let out = pull(&mut s, 0, 100);
    assert!(out.iter().all(|&x| (x - 0.1).abs() < 1e-6));
    assert_eq!(s.cue_state(0).unwrap().item, Some(0));

    // Press while playing: crossfade to the next song over 10 ms.
    s.fire_cue(0).unwrap();
    let out = pull(&mut s, 0, 1000);
    assert!((out[240] - 0.15).abs() < 0.01, "mid-crossfade {}", out[240]);
    assert!((out[900] - 0.2).abs() < 1e-6);
    assert_eq!(s.cue_state(0).unwrap().item, Some(1));

    // Without auto-advance it stops at the end of the song.
    pull(&mut s, 0, 5000);
    s.poll();
    assert!(!s.cue_state(0).unwrap().playing);
    s.fire_cue(0).unwrap();
    pull(&mut s, 0, 10);
    assert_eq!(s.cue_state(0).unwrap().item, Some(2));
    s.fire_cue(0).unwrap(); // wraps
    pull(&mut s, 0, 10);
    assert_eq!(s.cue_state(0).unwrap().item, Some(0));
}

#[test]
fn playlist_auto_advances_and_wraps() {
    let mut s = playlist_session(true, false);
    s.fire_cue(0).unwrap();
    let mut seen = Vec::new();
    for _ in 0..200 {
        let out = pull(&mut s, 0, 256);
        assert!(out.iter().all(|&x| x > 0.05), "no gap between songs");
        s.poll();
        let item = s.cue_state(0).unwrap().item;
        if seen.last() != Some(&item) {
            seen.push(item);
        }
    }
    assert!(s.cue_state(0).unwrap().playing, "keeps going: {seen:?}");
    // Each next song starts 60 ms (10 ms crossfade + 50 ms poll margin)
    // before the previous one ends, i.e. every 4800 - 2880 = 1920 frames
    // here; the outgoing song still plays to its end (checked above).
    let expected = 200 * 256 / 1920;
    assert!(
        seen.len().abs_diff(expected) <= 2,
        "{} songs, expected ~{expected}: {seen:?}",
        seen.len()
    );
    assert_eq!(
        &seen[..5],
        &[Some(0), Some(1), Some(2), Some(0), Some(1)],
        "{seen:?}"
    );
    s.stop_cue(0, 0.0).unwrap();
    pull(&mut s, 0, 1000);
    assert!(!s.cue_state(0).unwrap().playing, "stop stops both players");
}

#[test]
fn shuffle_plays_everything_once_per_round_without_repeats() {
    let mut s = playlist_session(false, true);
    let mut order = Vec::new();
    for _ in 0..30 {
        s.skip_cue(0).unwrap();
        pull(&mut s, 0, 10);
        order.push(s.cue_state(0).unwrap().item.unwrap());
    }
    for round in order.chunks(3) {
        let mut sorted = round.to_vec();
        sorted.sort();
        assert_eq!(sorted, vec![0, 1, 2], "{order:?}");
    }
    assert!(
        order.windows(2).all(|w| w[0] != w[1]),
        "no repeats: {order:?}"
    );
}

#[test]
fn playlist_missing_items_are_reported_and_skipped() {
    let backend = two_devices();
    let mut s = session(&backend);
    s.add_sound("a".into(), constant(0.1, 4_800));
    let mut c = CueDoc::simple(0, "P", "unused");
    c.clips.clear();
    c.playlist = Some(PlaylistDoc::new(vec!["/nope/gone.wav".into(), "a".into()]));
    s.set_cue(c).unwrap();
    assert!(s.problems().iter().any(|p| p.message.contains("gone.wav")));
    s.fire_cue(0).unwrap();
    pull(&mut s, 0, 10);
    assert_eq!(s.cue_state(0).unwrap().item, Some(1));
}

#[test]
fn fade_in_on_fire_and_pad_fade_out_length() {
    let backend = two_devices();
    let mut s = session(&backend);
    s.add_sound("a".into(), constant(1.0, 100_000));
    let mut c = cue(0, "a");
    c.fade_in = 0.01; // 480 frames
    c.fade_out = Some(0.02); // 960 frames
    c.retrigger = Retrigger::Fade;
    s.set_cue(c).unwrap();
    s.fire_cue(0).unwrap();
    let out = pull(&mut s, 0, 1000);
    assert!(out[0] < 0.01, "starts silent: {}", out[0]);
    assert!((out[240] - 0.5).abs() < 0.01, "{}", out[240]);
    assert!((out[600] - 1.0).abs() < 1e-6);

    s.fire_cue(0).unwrap(); // pressed again: fade over the pad's 20 ms
    let out = pull(&mut s, 0, 2000);
    assert!((out[480] - 0.5).abs() < 0.01, "{}", out[480]);
    assert!(out[960..].iter().all(|&x| x == 0.0));
}
