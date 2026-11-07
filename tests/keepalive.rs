//! An RC sends an empty 162 often enough that a quiet server does not
//! drop it. Loopback only. No real gserver and no archived client: the
//! stand-in speaks the plain-zlib RC login and closes if no 162 arrives
//! before its idle time.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

use seihai_core::protocol::{split_messages, write_zlib_frame, zlib_decompress};
use seihai_rc::rc::{LoginParams, RcKind, Session, PLI_PING};

fn read_frame(stream: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let mut lenb = [0u8; 2];
    stream.read_exact(&mut lenb)?;
    let n = u16::from_be_bytes(lenb) as usize;
    let mut body = vec![0u8; n];
    stream.read_exact(&mut body)?;
    zlib_decompress(&body)
}

#[test]
fn empty_162_arrives_before_the_idle_close() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let idle = Duration::from_millis(250);
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let _login = read_frame(&mut stream).unwrap();
        let sig = write_zlib_frame(&[25 + 0x20, b'\n']).unwrap();
        stream.write_all(&sig).unwrap();
        let start = Instant::now();
        while start.elapsed() < idle {
            match read_frame(&mut stream) {
                Ok(body) => {
                    let saw = split_messages(&body)
                        .iter()
                        .any(|m| m.id == PLI_PING && m.payload.is_empty());
                    if saw {
                        // Stay up past the idle time, as a server that
                        // accepted the keepalive would.
                        thread::sleep(idle);
                        return true;
                    }
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => return false,
            }
        }
        // Idle elapsed with no keepalive: drop the link.
        false
    });

    let mut session = Session::login(
        &addr,
        &LoginParams {
            kind: RcKind::Classic,
            token: "GSERV023".into(),
            account: "staff".into(),
            password: "pw".into(),
            key: 0,
            nickname: None,
        },
        Duration::from_secs(2),
    )
    .unwrap();
    session.set_keepalive_every(Duration::from_millis(40));
    let deadline = Instant::now() + Duration::from_millis(700);
    while Instant::now() < deadline && !session.is_closed() {
        if session.next_event(deadline).unwrap().is_none() {
            break;
        }
    }
    assert!(
        server.join().unwrap(),
        "the stand-in closed at its idle time; no empty 162 arrived"
    );
}
