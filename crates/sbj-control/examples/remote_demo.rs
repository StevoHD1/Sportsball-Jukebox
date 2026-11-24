//! Serves the remote page against a demo show on a virtual (silent)
//! output, for working on the page without the Mac app.
//!
//! ```sh
//! cargo run -p sbj-control --example remote_demo   # then open the URL it prints
//! ```

use std::sync::{Arc, Mutex};
use std::time::Duration;

use sbj_backend::offline::{OfflineBackend, OfflineDevice};
use sbj_control::remote::{DEFAULT_PORT, RemoteServer, Reply};
use sbj_engine::AudioBuffer;
use sbj_session::{CueDoc, Session, Sound};

fn tone(seconds: f64) -> Sound {
    let frames = (seconds * 48_000.0) as usize;
    let buffer = AudioBuffer::from_interleaved(48_000, 1, vec![0.0; frames]).unwrap();
    Sound::from_buffer(buffer, &[]).unwrap()
}

fn main() {
    let mut device = OfflineDevice::new("Virtual", 48_000, 2);
    device.is_default = true;
    let mut session = Session::new(OfflineBackend::with_devices(vec![device]));
    session.open_outputs();
    let banks = ["Pregame", "Game", "Postgame"].map(String::from).to_vec();
    session
        .set_show_settings("Home Opener", 12, 4, banks)
        .unwrap();
    let pads = [
        (0, "Charge!", "red", 6.0),
        (1, "Goal Horn", "orange", 4.0),
        (2, "Walk-up: #23", "blue", 30.0),
        (3, "Crowd Build", "teal", 20.0),
        (4, "DEFENSE!", "purple", 5.0),
        (5, "Let's Go!", "pink", 5.0),
        (6, "Timeout Music", "green", 90.0),
        (7, "Starting Lineups", "blue", 45.0),
        (8, "Anthem Intro", "gray", 12.0),
        (12, "Period Start", "green", 8.0),
    ];
    for (slot, title, color, seconds) in pads {
        let file = format!("{title}.wav");
        session.add_sound(file.clone(), tone(seconds));
        let mut cue = CueDoc::simple(slot, title, &file);
        cue.color = Some(color.into());
        cue.hotkey = Some(((b'1' + slot as u8) as char).to_string());
        session.set_cue(cue).unwrap();
    }
    session.fire_cue(6).unwrap();
    session.fire_cue(2).unwrap();

    let session = Arc::new(Mutex::new(session));
    let handler = {
        let session = Arc::clone(&session);
        Arc::new(move |method: &str, path: &str| {
            match session.lock().unwrap().handle_remote(method, path) {
                Ok(state) => Reply::json(state.to_string()),
                Err(e) => Reply::error(e.status, &e.message),
            }
        })
    };
    let server = RemoteServer::start(DEFAULT_PORT, "123456".into(), handler).unwrap();
    println!(
        "Remote demo: http://127.0.0.1:{}/#pin=123456  (PIN 123456)",
        server.port()
    );

    // Play the virtual output in real time.
    loop {
        std::thread::sleep(Duration::from_millis(50));
        let mut s = session.lock().unwrap();
        if let Some(stream) = s.output_stream_mut(0) {
            stream.render(2_400);
        }
        s.poll();
    }
}
