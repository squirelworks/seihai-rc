//! NPC-Control login against a fake server on an ephemeral loopback port.
//! The fake speaks the plain-zlib shapes in `docs/PROTOCOL.md`. It is not
//! the gserver-v2 fork; that pack was not available for this run.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use seihai_core::protocol::{split_messages, zlib_decompress};
use seihai_rc::link::Codec;
use seihai_rc::nc::{login_body, NcEvent, NcSession, NC_TOKEN};
use seihai_rc::rc::LoginError;
use seihai_rc::wire::message;

const TIMEOUT: Duration = Duration::from_secs(2);

fn read_raw_frame(s: &mut TcpStream) -> Vec<u8> {
    let mut len = [0u8; 2];
    s.read_exact(&mut len).unwrap();
    let mut f = vec![0u8; usize::from(u16::from_be_bytes(len))];
    s.read_exact(&mut f).unwrap();
    f
}

fn send_plain(s: &mut TcpStream, msgs: &[Vec<u8>]) {
    let mut body = Vec::new();
    for m in msgs {
        body.extend_from_slice(m);
        body.push(b'\n');
    }
    let mut codec = Codec::Plain;
    s.write_all(&codec.encode(&body).unwrap()).unwrap();
}

enum Reply {
    /// One plain-zlib message, then keep the link until the client closes.
    Message { id: u8, payload: Vec<u8> },
    /// Disconnect text, then stop.
    Refuse(&'static str),
    /// Accept and say nothing.
    Silent,
}

fn fake_nc(reply: Reply) -> (String, thread::JoinHandle<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let handle = thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        s.set_read_timeout(Some(TIMEOUT)).unwrap();
        let frame = read_raw_frame(&mut s);
        // Plain zlib: the body starts with a zlib header, not a gen-5
        // compression byte.
        assert_eq!(frame[0], 0x78, "NC login frame is zlib, got {frame:?}");
        let login = zlib_decompress(&frame).unwrap();
        match reply {
            Reply::Refuse(why) => {
                send_plain(&mut s, &[message(16, why.as_bytes())]);
            }
            Reply::Message { id, payload } => {
                send_plain(&mut s, &[message(id, &payload)]);
                let mut rest = Vec::new();
                let _ = s.read_to_end(&mut rest);
            }
            Reply::Silent => {
                let mut rest = Vec::new();
                let _ = s.read_to_end(&mut rest);
            }
        }
        login
    });
    (addr, handle)
}

#[test]
fn nc_login_round_trip_stays_plain_zlib() {
    let (addr, handle) = fake_nc(Reply::Message {
        id: 80,
        payload: b"start.nw".to_vec(),
    });
    let mut session = NcSession::login(&addr, "labnc", "labpw", TIMEOUT).unwrap();
    let ev = session
        .next_event(Instant::now() + TIMEOUT)
        .unwrap()
        .expect("level list");
    assert_eq!(
        ev,
        NcEvent::Message {
            id: 80,
            payload: b"start.nw".to_vec(),
        }
    );
    session.close();
    let login = handle.join().unwrap();
    assert_eq!(login, login_body("labnc", "labpw"));
    assert_eq!(&login[1..9], NC_TOKEN.as_bytes());
    let msgs = split_messages(&login);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].id, 3);
}

#[test]
fn refused_nc_login_reports_the_reason() {
    let (addr, handle) = fake_nc(Reply::Refuse("You do not have NC rights."));
    match NcSession::login(&addr, "labnc", "labpw", TIMEOUT) {
        Err(LoginError::Refused(why)) => assert_eq!(why, "You do not have NC rights."),
        Err(e) => panic!("wrong error: {e}"),
        Ok(_) => panic!("login should fail"),
    }
    handle.join().unwrap();
}

#[test]
fn quiet_open_link_counts_as_connected() {
    let (addr, handle) = fake_nc(Reply::Silent);
    let session = NcSession::login(&addr, "labnc", "labpw", Duration::from_millis(200)).unwrap();
    assert!(!session.is_closed());
    session.close();
    let login = handle.join().unwrap();
    assert_eq!(login, login_body("labnc", "labpw"));
}
