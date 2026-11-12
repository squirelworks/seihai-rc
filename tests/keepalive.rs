//! An RC sends an empty 162 often enough that a quiet server does not
//! drop it. Loopback only. No real gserver and no archived client: the
//! stand-in speaks the plain-zlib RC login and closes if no 162 arrives
//! before its idle time.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

use seihai_core::protocol::{split_messages, write_zlib_frame, zlib_decompress};
use seihai_rc::rc::{LoginParams, RcKind, Session, PLI_PONG};

fn read_frame(stream: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let mut lenb = [0u8; 2];
    stream.read_exact(&mut lenb)?;
    let n = u16::from_be_bytes(lenb) as usize;
    let mut body = vec![0u8; n];
    stream.read_exact(&mut body)?;
    zlib_decompress(&body)
}

fn staff_login(addr: &str) -> Session {
    Session::login(
        addr,
        &LoginParams {
            kind: RcKind::Classic,
            token: "GSERV023".into(),
            account: "staff".into(),
            password: "pw".into(),
            key: 0,
            nickname: None,
            identity: None,
        },
        Duration::from_secs(2),
    )
    .unwrap()
}

#[test]
fn empty_162_arrives_before_the_idle_close() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let idle = Duration::from_millis(400);
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let _login = read_frame(&mut stream).unwrap();
        let sig = write_zlib_frame(&[25 + 0x20, b'\n']).unwrap();
        stream.write_all(&sig).unwrap();
        // Short reads so the idle bound is checked between frames.
        // A one-second timeout hid the idle check until the read gave up.
        stream
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let start = Instant::now();
        let mut saw_ping = false;
        // Stay open after the empty 162 until the client closes. Dropping
        // here made the client's next keepalive a write on a dead socket:
        // macOS returns a broken pipe for that write, Linux often does not
        // until a later one.
        loop {
            if !saw_ping && start.elapsed() >= idle {
                break;
            }
            match read_frame(&mut stream) {
                Ok(body) => {
                    if split_messages(&body)
                        .iter()
                        .any(|m| m.id == PLI_PONG && m.payload.is_empty())
                    {
                        saw_ping = true;
                    }
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock
                            | std::io::ErrorKind::TimedOut
                            | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(_) => break,
            }
        }
        saw_ping
    });

    let mut session = staff_login(&addr);
    session.set_keepalive_every(Duration::from_millis(40));
    let deadline = Instant::now() + Duration::from_millis(700);
    while Instant::now() < deadline && !session.is_closed() {
        match session.next_event(deadline) {
            Ok(Some(_)) => {}
            Ok(None) => break,
            // A stand-in that gave up still has to fail the assertion
            // below, not panic on the platform's write-after-close error.
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::BrokenPipe
                        | std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::ConnectionAborted
                ) =>
            {
                break;
            }
            Err(e) => panic!("{e}"),
        }
    }
    session.close();
    assert!(
        server.join().unwrap(),
        "the stand-in closed at its idle time; no empty 162 arrived"
    );
}

#[test]
fn closed_peer_does_not_fail_the_keepalive_write() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let _login = read_frame(&mut stream).unwrap();
        let sig = write_zlib_frame(&[25 + 0x20, b'\n']).unwrap();
        stream.write_all(&sig).unwrap();
        // Take one keepalive, then drop. The following keepalive is a
        // write after the peer has gone.
        let _ = read_frame(&mut stream);
    });

    let mut session = staff_login(&addr);
    session.set_keepalive_every(Duration::from_millis(30));
    let deadline = Instant::now() + Duration::from_millis(500);
    let mut ended = false;
    while Instant::now() < deadline {
        match session.next_event(deadline) {
            Ok(Some(_)) => {}
            Ok(None) => {
                ended = true;
                break;
            }
            Err(e) => panic!("keepalive write after the peer closed: {e}"),
        }
    }
    server.join().unwrap();
    assert!(ended, "session did not notice the peer close");
    assert!(session.is_closed());
}
