use std::net::UdpSocket;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use rosc::{OscBundle, OscMessage, OscPacket, OscTime, OscType};
use sbj_control::osc::OscListener;
use sbj_control::{ControlMessage, Sink};
use sbj_session::control::OscArg;

#[test]
fn receives_messages_and_bundles() {
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let sink: Sink = Arc::new(move |m| {
        let _ = tx.lock().unwrap().send(m);
    });
    let listener = OscListener::start(0, sink).unwrap();
    let to = ("127.0.0.1", listener.port());
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();

    let message = |addr: &str, args| {
        OscPacket::Message(OscMessage {
            addr: addr.into(),
            args,
        })
    };
    let send = |p: &OscPacket| {
        socket
            .send_to(&rosc::encoder::encode(p).unwrap(), to)
            .unwrap()
    };
    send(&message("/sbj/fire/3", vec![OscType::Float(1.0)]));
    send(&OscPacket::Bundle(OscBundle {
        timetag: OscTime::from((0, 1)),
        content: vec![
            message("/sbj/bank/next", vec![]),
            message("/sbj/title", vec![OscType::String("Horn".into())]),
        ],
    }));
    send(&message("/x", vec![]));
    let _ = socket.send_to(b"garbage", to); // ignored

    let got: Vec<ControlMessage> = (0..4)
        .map(|_| rx.recv_timeout(Duration::from_secs(2)).unwrap())
        .collect();
    assert_eq!(
        got[0],
        ControlMessage::Osc {
            address: "/sbj/fire/3".into(),
            args: vec![OscArg::Float(1.0)]
        }
    );
    assert!(matches!(&got[1], ControlMessage::Osc { address, .. } if address == "/sbj/bank/next"));
    assert!(
        matches!(&got[2], ControlMessage::Osc { args, .. } if args == &[OscArg::Str("Horn".into())])
    );
    drop(listener); // stops promptly
}
