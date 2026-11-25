//! A small HTTP server for the phone/tablet remote page, on the local
//! network.
//!
//! Hand-written on `std` to avoid a web framework in a live-show app: one
//! listener thread, one short-lived thread per connection, `Connection:
//! close`, small size limits, and timeouts so a slow or hostile client
//! cannot tie it up. Everything under `/api/` needs the PIN (an
//! `X-SBJ-PIN` header); the page itself does not, and asks for it.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

pub const DEFAULT_PORT: u16 = 8765;

/// Largest request we read (headers + body).
const MAX_REQUEST: usize = 16 * 1024;
/// Concurrent connections allowed; more are refused.
const MAX_CONNECTIONS: usize = 32;
/// Pause before answering a wrong PIN.
const WRONG_PIN_DELAY: Duration = Duration::from_millis(300);

/// The page served at `/`.
pub const PAGE: &str = include_str!("remote.html");

/// What the app answers for an authorised `/api/...` request.
pub struct Reply {
    pub status: u16,
    pub content_type: &'static str,
    pub body: String,
}

impl Reply {
    pub fn json(body: String) -> Self {
        Self {
            status: 200,
            content_type: "application/json",
            body,
        }
    }

    pub fn error(status: u16, message: &str) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: format!("{{\"error\":{}}}", json_string(message)),
        }
    }
}

fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Handles `(method, path)` for authorised API requests, e.g.
/// `("GET", "/api/state")` or `("POST", "/api/fire/3")`.
pub type Handler = Arc<dyn Fn(&str, &str) -> Reply + Send + Sync>;

pub struct RemoteServer {
    port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl RemoteServer {
    /// Listens on all interfaces at `port` (0 = any free port).
    pub fn start(port: u16, pin: String, handler: Handler) -> io::Result<Self> {
        let listener = TcpListener::bind(("0.0.0.0", port))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = Arc::clone(&stop);
            std::thread::Builder::new()
                .name("sbj-remote".into())
                .spawn(move || accept_loop(listener, &stop, &Arc::new(pin), &handler))?
        };
        Ok(Self {
            port,
            stop,
            thread: Some(thread),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for RemoteServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn accept_loop(listener: TcpListener, stop: &AtomicBool, pin: &Arc<String>, handler: &Handler) {
    let active = Arc::new(AtomicUsize::new(0));
    // Wrong-PIN delays queue behind this, so parallel connections cannot
    // guess faster than one PIN per delay.
    let penalty = Arc::new(Mutex::new(()));
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                if active.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
                    continue; // dropped: closes the connection
                }
                active.fetch_add(1, Ordering::Relaxed);
                let (pin, handler, done, penalty) = (
                    Arc::clone(pin),
                    Arc::clone(handler),
                    Arc::clone(&active),
                    Arc::clone(&penalty),
                );
                let spawned = std::thread::Builder::new()
                    .name("sbj-remote-conn".into())
                    .spawn(move || {
                        let _ = serve(stream, &pin, &handler, &penalty);
                        done.fetch_sub(1, Ordering::Relaxed);
                    });
                if spawned.is_err() {
                    active.fetch_sub(1, Ordering::Relaxed);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// Constant-time comparison, so the PIN cannot be guessed by timing.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

fn serve(stream: TcpStream, pin: &str, handler: &Handler, penalty: &Mutex<()>) -> io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(stream.try_clone()?.take(MAX_REQUEST as u64));

    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut parts = request_line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return respond(stream, &Reply::error(400, "bad request"));
    };
    let (method, target) = (method.to_string(), target.to_string());

    let mut given_pin = None;
    let mut body_len = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            let value = value.trim();
            match name.trim().to_ascii_lowercase().as_str() {
                "x-sbj-pin" => given_pin = Some(value.to_string()),
                "content-length" => body_len = value.parse().unwrap_or(0).min(MAX_REQUEST),
                _ => {}
            }
        }
    }
    // Bodies are not used; drain any so the client sees a clean close.
    let mut sink = vec![0u8; body_len];
    let _ = reader.read_exact(&mut sink);

    let path = target.split('?').next().unwrap_or("/").to_string();
    let reply = match (method.as_str(), path.as_str()) {
        ("GET", "/" | "/index.html") => Reply {
            status: 200,
            content_type: "text/html; charset=utf-8",
            body: PAGE.to_string(),
        },
        (_, p) if p.starts_with("/api/") => {
            if given_pin.as_deref().is_some_and(|g| same(g, pin)) {
                handler(&method, &path)
            } else {
                // Slow down guessing (one at a time across connections).
                let _turn = penalty.lock().unwrap_or_else(|e| e.into_inner());
                std::thread::sleep(WRONG_PIN_DELAY);
                Reply::error(401, "wrong PIN")
            }
        }
        _ => Reply::error(404, "not found"),
    };
    respond(stream, &reply)
}

fn respond(mut stream: TcpStream, reply: &Reply) -> io::Result<()> {
    let reason = match reply.status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        reply.status,
        reply.content_type,
        reply.body.len()
    )?;
    stream.write_all(reply.body.as_bytes())?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_compare() {
        assert!(same("123456", "123456"));
        assert!(!same("123456", "123457"));
        assert!(!same("12345", "123456"));
    }

    #[test]
    fn json_strings_are_escaped() {
        assert_eq!(json_string("a\"b\\c\n"), "\"a\\\"b\\\\c\\u000a\"");
    }
}
