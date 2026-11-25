//! MIDI input from every connected device (CoreMIDI on macOS).

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod imp {
    use midir::{MidiInput, MidiInputConnection};
    use sbj_session::{ControlMessage, MidiMessage};

    use crate::Sink;

    pub struct MidiListener {
        _connections: Vec<MidiInputConnection<()>>,
        names: Vec<String>,
    }

    impl MidiListener {
        /// Connects to every MIDI input present now. Call again (dropping
        /// the old listener) to pick up devices plugged in later.
        pub fn start(sink: Sink) -> Result<Self, String> {
            let probe = MidiInput::new("Sportsball Jukebox").map_err(|e| e.to_string())?;
            let ports = probe.ports();
            let mut connections = Vec::new();
            let mut names = Vec::new();
            for port in &ports {
                let name = probe
                    .port_name(port)
                    .unwrap_or_else(|_| "MIDI input".into());
                let input = MidiInput::new("Sportsball Jukebox").map_err(|e| e.to_string())?;
                let sink = sink.clone();
                match input.connect(
                    port,
                    "sportsball-jukebox-in",
                    move |_, bytes, _| {
                        if let Some(message) = MidiMessage::parse(bytes) {
                            sink(ControlMessage::Midi(message));
                        }
                    },
                    (),
                ) {
                    Ok(connection) => {
                        connections.push(connection);
                        names.push(name);
                    }
                    // A device that will not open should not stop the rest.
                    Err(_) => continue,
                }
            }
            Ok(Self {
                _connections: connections,
                names,
            })
        }

        /// Names of the inputs being listened to.
        pub fn inputs(&self) -> &[String] {
            &self.names
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod imp {
    use crate::Sink;

    /// MIDI is not supported on this platform; listens to nothing.
    pub struct MidiListener;

    impl MidiListener {
        pub fn start(_sink: Sink) -> Result<Self, String> {
            Ok(Self)
        }

        pub fn inputs(&self) -> &[String] {
            &[]
        }
    }
}

pub use imp::MidiListener;
