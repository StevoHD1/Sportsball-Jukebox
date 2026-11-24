//! The remote's HTTP server over loopback.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

use sbj_control::remote::{PAGE, RemoteServer, Reply};

fn request(port: u16, raw: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.write_all(raw.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let status = response[9..12].parse().unwrap();
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default();
    (status, body)
}

fn get(port: u16, path: &str, pin: Option<&str>) -> (u16, String) {
    let pin = pin.map_or(String::new(), |p| format!("X-SBJ-PIN: {p}\r\n"));
    request(
        port,
        &format!("GET {path} HTTP/1.1\r\nHost: x\r\n{pin}\r\n"),
    )
}

fn server() -> (RemoteServer, Arc<Mutex<Vec<String>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&calls);
    let handler = Arc::new(move |method: &str, path: &str| {
        seen.lock().unwrap().push(format!("{method} {path}"));
        if path == "/api/missing" {
            Reply::error(404, "no")
        } else {
            Reply::json("{\"ok\":true}".into())
        }
    });
    (
        RemoteServer::start(0, "123456".into(), handler).unwrap(),
        calls,
    )
}

#[test]
fn serves_the_page_without_a_pin() {
    let (server, calls) = server();
    let (status, body) = get(server.port(), "/", None);
    assert_eq!(status, 200);
    assert_eq!(body, PAGE);
    assert_eq!(get(server.port(), "/elsewhere", None).0, 404);
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn api_needs_the_pin() {
    let (server, calls) = server();
    let port = server.port();
    assert_eq!(get(port, "/api/state", None).0, 401);
    assert_eq!(get(port, "/api/state", Some("000000")).0, 401);
    assert_eq!(get(port, "/api/state", Some("12345")).0, 401);
    assert!(calls.lock().unwrap().is_empty(), "handler never reached");

    let (status, body) = get(port, "/api/state?t=1", Some("123456"));
    assert_eq!((status, body.as_str()), (200, "{\"ok\":true}"));
    assert_eq!(get(port, "/api/missing", Some("123456")).0, 404);

    // A POST with a body; header names are case-insensitive.
    let (status, _) = request(
        port,
        "POST /api/fire/3 HTTP/1.1\r\nx-sbj-pin: 123456\r\nContent-Length: 2\r\n\r\n{}",
    );
    assert_eq!(status, 200);
    assert_eq!(
        *calls.lock().unwrap(),
        ["GET /api/state", "GET /api/missing", "POST /api/fire/3"]
    );
}

#[test]
fn junk_gets_a_400_and_the_server_stops_on_drop() {
    let (server, _) = server();
    let port = server.port();
    assert_eq!(request(port, "\r\n\r\n").0, 400);
    drop(server);
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
}
