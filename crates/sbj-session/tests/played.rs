//! "Played" marks: by hand, when pads fire, reset per page, kept out of
//! the show file and the undo history.

mod common;

use common::*;
use sbj_session::CueDoc;

/// Two pages of four; pads 0, 1 on page 1 and 4 on page 2.
fn setup() -> (
    sbj_backend::offline::OfflineBackend,
    sbj_session::Session<sbj_backend::offline::OfflineBackend>,
) {
    let backend = two_devices();
    let mut s = session(&backend);
    s.set_show_settings("T", 4, 4, vec!["One".into(), "Two".into()])
        .unwrap();
    s.add_sound("a".into(), constant(0.5, 48_000));
    for slot in [0, 1, 4] {
        s.set_cue(CueDoc::simple(slot, &format!("Pad {slot}"), "a"))
            .unwrap();
    }
    s.record_undo("Set Up");
    (backend, s)
}

fn played(s: &sbj_session::Session<sbj_backend::offline::OfflineBackend>) -> Vec<u32> {
    s.played().iter().copied().collect()
}

#[test]
fn marking_by_hand_and_resetting_a_page() {
    let (_b, mut s) = setup();
    s.set_played(0, true);
    s.set_played(4, true);
    s.set_played(2, true); // empty pad: ignored
    assert_eq!(played(&s), [0, 4]);
    s.set_played(0, false);
    assert_eq!(played(&s), [4]);
    s.set_played(1, true);
    s.reset_played(Some(1)); // page 2 only
    assert_eq!(played(&s), [1]);
    s.set_played(4, true);
    s.reset_played(None);
    assert!(played(&s).is_empty());
}

#[test]
fn firing_marks_pads_once_a_played_colour_is_set() {
    let (_b, mut s) = setup();
    s.fire_cue(0).unwrap();
    assert!(played(&s).is_empty(), "off until a played colour is chosen");

    s.set_played_style(Some("#555555".into()), true);
    assert!(s.is_dirty());
    s.fire_cue(1).unwrap();
    assert_eq!(played(&s), [1]);
    // From the remote too, and the remote shows it.
    s.handle_remote("POST", "/api/fire/4").unwrap();
    let state = s.handle_remote("GET", "/api/state").unwrap();
    assert_eq!(state["played_color"], "#555555");
    assert_eq!(state["pads"][1]["played"], true);
    assert_eq!(played(&s), [1, 4]);

    // Marking on fire can be turned off while keeping the colour.
    s.set_played_style(Some("#555555".into()), false);
    s.fire_cue(0).unwrap();
    assert_eq!(played(&s), [1, 4]);
}

#[test]
fn marks_follow_pads_and_stay_out_of_the_show_and_undo() {
    let (_b, mut s) = setup();
    s.set_played(1, true);
    s.move_cue(1, 3).unwrap();
    assert_eq!(played(&s), [3], "follows the pad");
    s.record_undo("Move");
    s.set_played(0, true);
    assert!(!s.record_undo("Played"), "marks aren't a show edit");
    assert!(!s.show().to_json().contains("played\": ["));
    s.remove_cue(3).unwrap();
    assert_eq!(played(&s), [0], "a cleared pad isn't played");
    s.set_played_style(Some("gray".into()), true);
    let back = sbj_session::Show::from_json(&s.show().to_json()).unwrap();
    assert_eq!(back.played_color.as_deref(), Some("gray"));
    s.new_show();
    assert!(played(&s).is_empty(), "a new show starts fresh");
}
