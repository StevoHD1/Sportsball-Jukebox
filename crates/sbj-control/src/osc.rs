//! OSC over UDP. See `sbj_session::control` for the address space.

use std::io;
use std::net::UdpSocket;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use rosc::{OscPacket, OscType};
use sbj_session::control::OscArg;

use crate::{ControlMessage, Sink};

/// Default port. (QLab uses 53000; 53100 avoids clashing with it.)
pub const DEFAULT_PORT: u16 = 53100;

pub struct OscListener {
    port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl OscListener {
    /// Listens on all interfaces at `port` (0 = any free port).
    pub fn start(port: u16, sink: Sink) -> io::Result<Self> {
        let socket = UdpSocket::bind(("0.0.0.0", port))?;
        // Wake regularly so dropping the listener is prompt.
        socket.set_read_timeout(Some(Duration::from_millis(200)))?;
        let port = socket.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = Arc::clone(&stop);
            std::thread::Builder::new()
                .name("sbj-osc".into())
                .spawn(move || run(socket, &stop, &sink))?
        };
        Ok(Self {
            port,
            stop,
            thread: Some(thread),
        })
    }

    /// The UDP port actually bound.
    pub fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for OscListener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(socket: UdpSocket, stop: &AtomicBool, sink: &Sink) {
    let mut buf = [0u8; 4096];
    while !stop.load(Ordering::Relaxed) {
        let n = match socket.recv(&mut buf) {
            Ok(n) => n,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            // Transient errors (e.g. ICMP port unreachable on some
            // platforms) must not kill the listener.
            Err(_) => continue,
        };
        if let Ok((_, packet)) = rosc::decoder::decode_udp(&buf[..n]) {
            deliver(packet, sink);
        }
    }
}

fn deliver(packet: OscPacket, sink: &Sink) {
    match packet {
        OscPacket::Message(message) => sink(ControlMessage::Osc {
            address: message.addr,
            args: message.args.into_iter().map(convert).collect(),
        }),
        OscPacket::Bundle(bundle) => {
            for packet in bundle.content {
                deliver(packet, sink);
            }
        }
    }
}

fn convert(arg: OscType) -> OscArg {
    match arg {
        OscType::Int(i) => OscArg::Int(i64::from(i)),
        OscType::Long(i) => OscArg::Int(i),
        OscType::Float(f) => OscArg::Float(f64::from(f)),
        OscType::Double(f) => OscArg::Float(f),
        OscType::Bool(b) => OscArg::Int(i64::from(b)),
        OscType::String(s) => OscArg::Str(s),
        _ => OscArg::Other,
    }
}
