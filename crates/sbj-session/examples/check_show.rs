//! Loads a show file with the offline backend and prints its pre-game
//! check. Useful for validating show files without audio hardware.
//!
//!     cargo run -p sbj-session --example check_show -- path/to/show.sbjshow

use sbj_backend::offline::OfflineBackend;
use sbj_session::{Session, Show, decode_files};

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: check_show <show.sbjshow>");
        std::process::exit(2);
    };
    let path = std::path::PathBuf::from(path);
    let show = match Show::load(&path) {
        Ok(show) => show,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let decoded = decode_files(&show.files(), path.parent(), &[48_000]);
    let mut session = Session::new(OfflineBackend::default());
    session.install_show(show, Some(path), decoded);
    let report = session.preflight();
    for check in &report.checks {
        println!("{:?}: {}", check.level, check.message);
    }
    std::process::exit(if report.ready() { 0 } else { 1 });
}
