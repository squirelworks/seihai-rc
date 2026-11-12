//! List, get and put against a fake gserver on an ephemeral loopback port.
//!
//! The fake speaks the layout from the 2026-11-01 loopback run
//! (`docs/PROTOCOL.md`). It does not use the crate's builders: it writes
//! the bytes itself, and it reads a put back itself.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use seihai_core::protocol::{read_gint3, zlib_decompress};
use seihai_rc::files::{FileError, LARGE_CHUNK_BYTES, MAX_FILE_BYTES, MAX_LARGE_FILE_BYTES};
use seihai_rc::link::Codec;
use seihai_rc::rc::{Event, LoginParams, RcKind, Session};
use seihai_rc::wire::{gchar, message, push_gint5, push_gstring, read_gstring, text};

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

/// Id 67, then id 66: GSTRING folder, then each file as a space, a GCHAR
/// length, a GSTRING name, a GSTRING rights, a GINT5 size and a GINT5 time.
fn dir_reply(folder: &str, names: &[&str]) -> Vec<u8> {
    let mut notice = with_nl(message(
        67,
        format!("Folder changed to {folder}").as_bytes(),
    ));
    let mut p = Vec::new();
    push_gstring(&mut p, folder.as_bytes());
    for name in names {
        let mut rec = Vec::new();
        push_gstring(&mut rec, name.as_bytes());
        push_gstring(&mut rec, b"rw");
        push_gint5(&mut rec, 0);
        push_gint5(&mut rec, 0);
        p.push(b' ');
        p.push(gchar(rec.len() as u8));
        p.extend_from_slice(&rec);
    }
    notice.extend(with_nl(message(66, &p)));
    notice
}

fn malformed_list_body() -> Vec<u8> {
    // Folder length 10, and only five bytes follow.
    let mut p = vec![gchar(10)];
    p.extend_from_slice(b"short");
    with_nl(message(66, &p))
}

/// Id 100 raw block. Inside: id 102, GINT5 time, GSTRING name, bytes, and
/// the line feed the raw length counts.
fn file_reply(name: &str, data: &[u8]) -> Vec<u8> {
    let mut packet = vec![gchar(102)];
    push_gint5(&mut packet, 0);
    push_gstring(&mut packet, name.as_bytes());
    packet.extend_from_slice(data);
    let n = packet.len() + 1;
    let mut body = vec![gchar(100)];
    body.extend_from_slice(&gint3(n));
    body.push(b'\n');
    body.extend_from_slice(&packet);
    body.push(b'\n');
    body
}

/// A large file the way the gserver-v2 fork sends it: id 68 with the name,
/// id 84 with a GINT5 size, one id 102 block per 32000 bytes, then id 69.
/// `size` is what id 84 says, so a test can make it disagree.
fn large_file_reply(name: &str, data: &[u8], size: u32) -> Vec<u8> {
    let mut out = with_nl(message(68, name.as_bytes()));
    let mut p = Vec::new();
    push_gint5(&mut p, size);
    out.extend(with_nl(message(84, &p)));
    for piece in data.chunks(32000) {
        out.extend(file_reply(name, piece));
    }
    out.extend(with_nl(message(69, name.as_bytes())));
    out
}

/// Bytes that do not repeat on a 32000 boundary, so a lost or doubled piece
/// shows.
fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

fn text_reply(words: &[u8]) -> Vec<u8> {
    with_nl(message(67, words))
}

struct Seen {
    lists: Vec<String>,
    gets: Vec<String>,
    puts: Vec<(String, Vec<u8>)>,
    /// A large put after id 155: the name and the pieces so far.
    large: Option<(String, Vec<u8>)>,
    /// How many id 93 blocks each large put came in.
    large_pieces: Vec<usize>,
}

fn take_put(payload: &[u8]) -> (String, Vec<u8>) {
    assert!(!payload.is_empty(), "put name length is one byte");
    let mut i = 0;
    let name = read_gstring(payload, &mut i).expect("put name is a GSTRING");
    assert_eq!(i, 1 + name.len(), "name length is one byte, not a GSHORT");
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

fn reply(m: &seihai_core::protocol::Message, seen: &mut Seen) -> Option<Vec<u8>> {
    match m.id {
        90 => {
            // The payload is the folder. A length byte would make `levels`
            // into `&levels` and this match would miss.
            let dir = text(&m.payload);
            seen.lists.push(dir.clone());
            Some(match dir.as_str() {
                "" => dir_reply("", &["levels"]),
                "levels" => dir_reply("levels", &["a.nw", "b.nw"]),
                "bad" => malformed_list_body(),
                _ => text_reply(b"no such directory"),
            })
        }
        92 => {
            let path = text(&m.payload);
            seen.gets.push(path.clone());
            Some(match path.as_str() {
                "levels/a.nw" => file_reply("a.nw", LEVEL),
                "levels/empty.nw" => file_reply("empty.nw", b""),
                "huge.bin" => file_reply("huge.bin", &vec![0xAB; MAX_FILE_BYTES + 1]),
                // 40000 bytes: under MAX_FILE_BYTES, but the fork sends
                // anything over 32000 in pieces.
                "mid.png" => large_file_reply("mid.png", &pattern(40_000), 40_000),
                "big.png" => large_file_reply("big.png", &pattern(100_000), 100_000),
                "short.png" => large_file_reply("short.png", &pattern(40_000), 50_000),
                "liar.png" => large_file_reply("liar.png", &pattern(40_000), 30_000),
                "vast.png" => large_file_reply("vast.png", b"", MAX_LARGE_FILE_BYTES as u32 + 1),
                _ => text_reply(b"no such file"),
            })
        }
        155 => {
            assert!(seen.large.is_none(), "a large put started inside another");
            seen.large = Some((text(&m.payload), Vec::new()));
            seen.large_pieces.push(0);
            None
        }
        156 => {
            let name = text(&m.payload);
            let (started, data) = seen.large.take().expect("156 without 155");
            assert_eq!(started, name, "155 and 156 name the same file");
            seen.puts.push((name.clone(), data));
            Some(text_reply(format!("Uploaded large file {name}").as_bytes()))
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
            large: None,
            large_pieces: Vec::new(),
        };
        while let Ok(frame) = read_frame(&mut stream) {
            let body = codec.decode(&frame).unwrap();
            if body.first().map(|b| b.wrapping_sub(0x20)) == Some(50) {
                let (id, payload) = split_client_raw(&body);
                assert_eq!(id, 93);
                let put = take_put(&payload);
                if let Some((name, data)) = seen.large.as_mut() {
                    // The fork keeps a piece of a large put and does not
                    // answer it.
                    assert_eq!(*name, put.0, "every piece names the large file");
                    data.extend_from_slice(&put.1);
                    *seen.large_pieces.last_mut().unwrap() += 1;
                    continue;
                }
                let ack = format!("Uploaded file {}", put.0);
                if put.0 == "levels/nope.nw" {
                    send_body(&mut stream, &mut codec, &text_reply(b"not stored"));
                } else {
                    send_body(&mut stream, &mut codec, &text_reply(ack.as_bytes()));
                }
                seen.puts.push(put);
                continue;
            }
            for m in seihai_core::protocol::split_messages(&body) {
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
        identity: None,
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
    session.put_file("levels/a.nw", put, TIMEOUT).unwrap();
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
    match session.list_dir("missing", TIMEOUT) {
        Err(FileError::Refused(why)) => assert_eq!(why, "no such directory"),
        other => panic!("expected a refusal, got {other:?}"),
    }
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
    let big = vec![1u8; MAX_LARGE_FILE_BYTES + 1];
    match session.put_file("levels/big.nw", &big, TIMEOUT) {
        Err(FileError::TooLarge { len, max }) => {
            assert_eq!(len, big.len());
            assert_eq!(max, MAX_LARGE_FILE_BYTES);
        }
        other => panic!("expected too large, got {other:?}"),
    }
    match session.put_file("levels/nope.nw", b"x", TIMEOUT) {
        Err(FileError::Refused(why)) => assert_eq!(why, "not stored"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    session.close();
    let seen = handle.join().unwrap();
    assert_eq!(
        seen.puts,
        [("levels/nope.nw".into(), b"x".to_vec())],
        "a refused put of a too-large file must not be sent"
    );
    assert_eq!(seen.gets, ["missing.nw", "huge.bin"]);
}

#[test]
fn large_files_travel_in_pieces() {
    let (addr, handle) = fake_file_server();
    let mut session = Session::login(&addr, &params(RcKind::Gen5), TIMEOUT).unwrap();
    assert_eq!(
        session.get_file("mid.png", TIMEOUT).unwrap(),
        pattern(40_000)
    );
    assert_eq!(
        session.get_file("big.png", TIMEOUT).unwrap(),
        pattern(100_000)
    );
    // A put of exactly MAX_FILE_BYTES is still one block; one byte more is
    // a large put.
    let edge = pattern(MAX_FILE_BYTES);
    session.put_file("edge.png", &edge, TIMEOUT).unwrap();
    let big = pattern(MAX_FILE_BYTES + 1);
    session.put_file("big.png", &big, TIMEOUT).unwrap();
    let exact = pattern(LARGE_CHUNK_BYTES * 3);
    session.put_file("exact.png", &exact, TIMEOUT).unwrap();
    // Queued events before the file are still there afterwards.
    assert_eq!(
        session.next_event(Instant::now()).unwrap(),
        Some(Event::Signature)
    );
    session.close();
    let seen = handle.join().unwrap();
    assert_eq!(
        seen.puts,
        [
            ("edge.png".into(), edge),
            ("big.png".into(), big),
            ("exact.png".into(), exact),
        ]
    );
    // 61441 bytes is two pieces; 96000 is three, with no empty fourth.
    assert_eq!(seen.large_pieces, [2, 3]);
}

#[test]
fn large_file_that_disagrees_with_its_size_is_an_error() {
    let (addr, handle) = fake_file_server();
    let mut session = Session::login(&addr, &params(RcKind::Gen5), TIMEOUT).unwrap();
    assert!(matches!(
        session.get_file("short.png", TIMEOUT),
        Err(FileError::Malformed(_))
    ));
    session.close();
    handle.join().unwrap();

    let (addr, handle) = fake_file_server();
    let mut session = Session::login(&addr, &params(RcKind::Gen5), TIMEOUT).unwrap();
    assert!(matches!(
        session.get_file("liar.png", TIMEOUT),
        Err(FileError::Malformed(_))
    ));
    session.close();
    handle.join().unwrap();

    let (addr, handle) = fake_file_server();
    let mut session = Session::login(&addr, &params(RcKind::Gen5), TIMEOUT).unwrap();
    match session.get_file("vast.png", TIMEOUT) {
        Err(FileError::TooLarge { len, max }) => {
            assert_eq!(len, MAX_LARGE_FILE_BYTES + 1);
            assert_eq!(max, MAX_LARGE_FILE_BYTES);
        }
        other => panic!("expected too large, got {other:?}"),
    }
    session.close();
    handle.join().unwrap();
}
