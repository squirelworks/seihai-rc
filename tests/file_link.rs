//! List, get and put against a fake gserver on an ephemeral loopback port.
//!
//! The fake follows the layout in `docs/PROTOCOL.md` (payload fields there
//! are still a lab check). It does not use the crate's builders: it writes
//! the bytes itself, and it reads a put back itself.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use seihai_core::protocol::{read_gint3, split_messages, zlib_decompress, Message};
use seihai_rc::files::{FileError, MAX_FILE_BYTES};
use seihai_rc::link::Codec;
use seihai_rc::rc::{Event, LoginParams, RcKind, Session};
use seihai_rc::wire::{gchar, message, push_gstring, read_gstring, text};

const TIMEOUT: Duration = Duration::from_secs(5);
const LEVEL: &[u8] = b"GLEVNW01\nBOARD \xff\n";

fn gint3(n: usize) -> [u8; 3] {
    [
        gchar(((n >> 14) & 0x7F) as u8),
        gchar(((n >> 7) & 0x7F) as u8),
        gchar((n & 0x7F) as u8),
    ]
}

fn read_frame(s: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut len = [0u8; 2];
    s.read_exact(&mut len)?;
    let mut f = vec![0u8; usize::from(u16::from_be_bytes(len))];
    s.read_exact(&mut f)?;
    Ok(f)
}

fn send_body(s: &mut TcpStream, codec: &mut Codec, body: &[u8]) {
    s.write_all(&codec.encode(body).unwrap()).unwrap();
    s.flush().unwrap();
}

fn with_nl(mut msg: Vec<u8>) -> Vec<u8> {
    msg.push(b'\n');
    msg
}

/// Id 65, GSHORT count, GSTRING names. Count 2 with one name is the bad case.
fn dir_list_body(names: &[&str]) -> Vec<u8> {
    let n = names.len() as u16;
    let mut p = vec![gchar((n >> 7) as u8), gchar((n & 0x7F) as u8)];
    for name in names {
        push_gstring(&mut p, name.as_bytes());
    }
    with_nl(message(65, &p))
}

fn malformed_list_body() -> Vec<u8> {
    // Count says 2. The only name says it is 10 bytes long and 5 follow.
    let mut p = vec![gchar(0), gchar(2), gchar(10)];
    p.extend_from_slice(b"short");
    with_nl(message(65, &p))
}

/// Id 100 raw block, inner id 66, GSTRING name, GINT3 size, bytes.
fn file_reply(name: &str, data: &[u8]) -> Vec<u8> {
    let mut inner = vec![gchar(66)];
    push_gstring(&mut inner, name.as_bytes());
    inner.extend_from_slice(&gint3(data.len()));
    inner.extend_from_slice(data);
    let mut body = vec![gchar(100)];
    body.extend_from_slice(&gint3(inner.len()));
    body.push(b'\n');
    body.extend_from_slice(&inner);
    body
}

fn text_reply(words: &[u8]) -> Vec<u8> {
    with_nl(message(67, words))
}

struct Seen {
    lists: Vec<String>,
    gets: Vec<String>,
    puts: Vec<(String, Vec<u8>)>,
}

fn gstring_text(payload: &[u8]) -> String {
    let mut i = 0;
    text(&read_gstring(payload, &mut i).unwrap())
}

fn take_put(payload: &[u8]) -> (String, Vec<u8>) {
    let mut i = 0;
    let name = read_gstring(payload, &mut i).unwrap();
    let size = read_gint3(&payload[i..i + 3]).unwrap();
    i += 3;
    assert_eq!(payload.len(), i + size, "put size does not match");
    (text(&name), payload[i..].to_vec())
}

fn split_client_raw(body: &[u8]) -> (u8, Vec<u8>) {
    assert_eq!(
        body[0].wrapping_sub(0x20),
        50,
        "put is an outbound raw block"
    );
    let n = read_gint3(&body[1..4]).unwrap();
    assert_eq!(body[4], b'\n');
    assert_eq!(body.len(), 5 + n);
    let inner = &body[5..];
    (inner[0].wrapping_sub(0x20), inner[1..].to_vec())
}

fn reply(m: &Message, seen: &mut Seen) -> Option<Vec<u8>> {
    match m.id {
        90 => {
            let dir = gstring_text(&m.payload);
            seen.lists.push(dir.clone());
            Some(match dir.as_str() {
                "" => dir_list_body(&["levels"]),
                "levels" => dir_list_body(&["a.nw", "b.nw"]),
                "bad" => malformed_list_body(),
                _ => text_reply(b"no such directory"),
            })
        }
        92 => {
            let path = gstring_text(&m.payload);
            seen.gets.push(path.clone());
            Some(match path.as_str() {
                "levels/a.nw" => file_reply("levels/a.nw", LEVEL),
                "levels/empty.nw" => file_reply("levels/empty.nw", b""),
                "huge.bin" => file_reply("huge.bin", &vec![0xAB; MAX_FILE_BYTES + 1]),
                _ => text_reply(b"no such file"),
            })
        }
        _ => None,
    }
}

fn fake_file_server() -> (String, JoinHandle<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(TIMEOUT)).unwrap();
        let login = zlib_decompress(&read_frame(&mut stream).unwrap()).unwrap();
        let mut codec = match login[0] - 0x20 {
            6 => Codec::gen5(login[1] - 0x20),
            1 => Codec::Plain,
            other => panic!("unexpected login type {other}"),
        };
        send_body(&mut stream, &mut codec, &with_nl(message(25, &[gchar(73)])));
        let mut seen = Seen {
            lists: Vec::new(),
            gets: Vec::new(),
            puts: Vec::new(),
        };
        while let Ok(frame) = read_frame(&mut stream) {
            let body = codec.decode(&frame).unwrap();
            if body.first().map(|b| b.wrapping_sub(0x20)) == Some(50) {
                let (id, payload) = split_client_raw(&body);
                assert_eq!(id, 93);
                seen.puts.push(take_put(&payload));
                continue;
            }
            for m in split_messages(&body) {
                if let Some(out) = reply(&m, &mut seen) {
                    send_body(&mut stream, &mut codec, &out);
                }
            }
        }
        seen
    });
    (addr, handle)
}

fn params(kind: RcKind) -> LoginParams {
    LoginParams {
        kind,
        token: kind.default_token().into(),
        account: "labrc".into(),
        password: "labpw".into(),
        key: 0x35,
        nickname: None,
    }
}

fn run_happy_path(kind: RcKind) {
    let (addr, handle) = fake_file_server();
    let mut session = Session::login(&addr, &params(kind), TIMEOUT).unwrap();
    assert_eq!(session.list_dir("", TIMEOUT).unwrap(), ["levels"]);
    assert_eq!(
        session.list_dir("levels", TIMEOUT).unwrap(),
        ["a.nw", "b.nw"]
    );
    assert_eq!(session.get_file("levels/a.nw", TIMEOUT).unwrap(), LEVEL);
    assert_eq!(session.get_file("levels/empty.nw", TIMEOUT).unwrap(), b"");
    let put = b"replaced\nlevel\0";
    session.put_file("levels/a.nw", put).unwrap();
    assert_eq!(
        session.next_event(Instant::now()).unwrap(),
        Some(Event::Signature)
    );
    session.close();
    let seen = handle.join().unwrap();
    assert_eq!(seen.lists, ["", "levels"]);
    assert_eq!(seen.gets, ["levels/a.nw", "levels/empty.nw"]);
    assert_eq!(seen.puts, [("levels/a.nw".into(), put.to_vec())]);
}

#[test]
fn gen5_list_get_and_put() {
    run_happy_path(RcKind::Gen5);
}

#[test]
fn classic_list_get_and_put() {
    run_happy_path(RcKind::Classic);
}

#[test]
fn malformed_lengths_and_too_large_file_do_not_panic() {
    let (addr, handle) = fake_file_server();
    let mut session = Session::login(&addr, &params(RcKind::Gen5), TIMEOUT).unwrap();
    assert!(matches!(
        session.list_dir("bad", TIMEOUT),
        Err(FileError::Malformed(_))
    ));
    match session.get_file("missing.nw", TIMEOUT) {
        Err(FileError::Refused(why)) => assert_eq!(why, "no such file"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    match session.get_file("huge.bin", TIMEOUT) {
        Err(FileError::TooLarge { len, max }) => {
            assert_eq!(len, MAX_FILE_BYTES + 1);
            assert_eq!(max, MAX_FILE_BYTES);
        }
        other => panic!("expected too large, got {other:?}"),
    }
    let big = vec![1u8; MAX_FILE_BYTES + 1];
    match session.put_file("levels/big.nw", &big) {
        Err(FileError::TooLarge { len, max }) => {
            assert_eq!(len, big.len());
            assert_eq!(max, MAX_FILE_BYTES);
        }
        other => panic!("expected too large, got {other:?}"),
    }
    session.close();
    let seen = handle.join().unwrap();
    assert!(seen.puts.is_empty(), "a refused put must not be sent");
    assert_eq!(seen.gets, ["missing.nw", "huge.bin"]);
}
