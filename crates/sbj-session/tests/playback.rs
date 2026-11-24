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
