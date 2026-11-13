//! Login and player list against small fake servers on ephemeral loopback
//! ports. The fakes follow the wire shapes in `docs/PROTOCOL.md`; they check
//! what the RC sends and answer the way the gserver-v2 fork did in the lab.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use std::process::Command;

use seihai_core::protocol::seclogin::{self, ClientHello, ServerIdentity};
use seihai_core::protocol::{split_messages, zlib_decompress, Message};
use seihai_rc::link::Codec;
use seihai_rc::listserver::{fetch_rc_list, fetch_rc_list_secure, pick, ListError};
use seihai_rc::rc::{Event, LoginError, LoginParams, RcKind, Session};
use seihai_rc::wire::{gchar, message, push_gstring};

const TIMEOUT: Duration = Duration::from_secs(5);

fn read_raw_frame(s: &mut TcpStream) -> Vec<u8> {
    let mut len = [0u8; 2];
    s.read_exact(&mut len).unwrap();
    let mut f = vec![0u8; usize::from(u16::from_be_bytes(len))];
    s.read_exact(&mut f).unwrap();
    f
}

fn send_msgs(s: &mut TcpStream, codec: &mut Codec, msgs: &[Vec<u8>]) {
    let mut body = Vec::new();
    for m in msgs {
        body.extend_from_slice(m);
        body.push(b'\n');
    }
    s.write_all(&codec.encode(&body).unwrap()).unwrap();
}

fn addplayer(id: u16, account: &str, level: &str, nick: &str) -> Vec<u8> {
    let mut p = vec![gchar((id >> 7) as u8), gchar((id & 0x7F) as u8)];
    push_gstring(&mut p, account.as_bytes());
    p.push(gchar(20));
    push_gstring(&mut p, level.as_bytes());
    p.push(gchar(53));
    p.push(gchar(0));
    p.push(gchar(0));
    push_gstring(&mut p, nick.as_bytes());
    p.push(gchar(82));
    push_gstring(&mut p, account.as_bytes());
    message(55, &p)
}

/// What the fake gserver saw.
#[derive(Debug)]
struct Seen {
    login: Vec<u8>,
    after: Vec<Message>,
}

/// A one-connection fake gserver. `accept` decides between the welcome
/// burst and a refusal.
fn fake_gserver(accept: bool) -> (String, thread::JoinHandle<Seen>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap().to_string();
    let h = thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        s.set_read_timeout(Some(TIMEOUT)).unwrap();
        let login = zlib_decompress(&read_raw_frame(&mut s)).unwrap();
        let mut codec = match login[0] - 0x20 {
            6 => Codec::gen5(login[1] - 0x20),
            1 => Codec::Plain,
            t => panic!("unexpected login type {t}"),
        };
        if !accept {
            send_msgs(
                &mut s,
                &mut codec,
                &[message(16, b"You do not have RC rights.")],
            );
            return Seen {
                login,
                after: Vec::new(),
            };
        }
        send_msgs(
            &mut s,
            &mut codec,
            &[
                message(25, &[gchar(73)]),
                message(74, b"Welcome to the lab"),
            ],
        );
        // The list comes in a later frame, as it may on a real server.
        thread::sleep(Duration::from_millis(50));
        send_msgs(
            &mut s,
            &mut codec,
            &[
                addplayer(2, "alice", "start.nw", "Alice"),
                addplayer(3, "bob", "cave.nw", "Bob"),
                addplayer(200, "staff", " ", "*staff"),
            ],
        );
        thread::sleep(Duration::from_millis(50));
        send_msgs(&mut s, &mut codec, &[message(56, &[gchar(0), gchar(3)])]);
        // Collect what the RC sends after the login until it closes.
        let mut after = Vec::new();
        let mut rest = Vec::new();
        let _ = s.read_to_end(&mut rest);
        let mut i = 0;
        while i + 2 <= rest.len() {
            let n = usize::from(u16::from_be_bytes([rest[i], rest[i + 1]]));
            let body = codec.decode(&rest[i + 2..i + 2 + n]).unwrap();
            after.extend(split_messages(&body));
            i += 2 + n;
        }
        Seen { login, after }
    });
    (addr, h)
}

fn params(kind: RcKind) -> LoginParams {
    LoginParams {
        kind,
        token: kind.default_token().into(),
        account: "labrc".into(),
        password: "labpw".into(),
        key: 0x35,
        nickname: Some("Lab RC".into()),
        identity: None,
    }
}

fn run_player_list(kind: RcKind) {
    let (addr, h) = fake_gserver(true);
    let mut s = Session::login(&addr, &params(kind), TIMEOUT).unwrap();
    s.settle(Duration::from_millis(300), Instant::now() + TIMEOUT)
        .unwrap();
    let ids: Vec<u16> = s.players.players.keys().copied().collect();
    assert_eq!(ids, [2, 200], "bob (3) left again");
    assert_eq!(s.players.players[&2].nickname, "Alice");
    assert_eq!(s.players.players[&2].level, "start.nw");
    assert!(s.players.players[&200].is_rc());
    s.send_rc_chat("hello").unwrap();
    s.close();
    let seen = h.join().unwrap();
    let type_byte = seen.login[0] - 0x20;
    assert_eq!(type_byte, kind.login_type());
    let token_at = if kind == RcKind::Gen5 { 2 } else { 1 };
    assert_eq!(
        &seen.login[token_at..token_at + 8],
        kind.default_token().as_bytes()
    );
    assert!(seen.login.ends_with(b"%labrc%labpw\n"));
    // Nickname, then the chat line.
    assert_eq!(seen.after.len(), 2, "{:?}", seen.after);
    assert_eq!(seen.after[0].id, 2);
    assert_eq!(seen.after[0].payload, b" &Lab RC");
    assert_eq!(seen.after[1].id, 79);
    assert_eq!(seen.after[1].payload, b"hello");
}

#[test]
fn gen5_login_and_player_list() {
    run_player_list(RcKind::Gen5);
}

#[test]
fn classic_login_and_player_list() {
    run_player_list(RcKind::Classic);
}

#[test]
fn refused_login_reports_the_reason() {
    let (addr, h) = fake_gserver(false);
    match Session::login(&addr, &params(RcKind::Gen5), TIMEOUT) {
        Err(LoginError::Refused(why)) => assert_eq!(why, "You do not have RC rights."),
        Err(e) => panic!("wrong error: {e}"),
        Ok(_) => panic!("login should fail"),
    }
    h.join().unwrap();
}

#[test]
fn events_keep_server_order() {
    let (addr, h) = fake_gserver(true);
    let mut s = Session::login(&addr, &params(RcKind::Gen5), TIMEOUT).unwrap();
    let evs = s
        .settle(Duration::from_millis(300), Instant::now() + TIMEOUT)
        .unwrap();
    assert_eq!(evs[0], Event::Signature);
    assert_eq!(evs[1], Event::RcChat("Welcome to the lab".into()));
    assert!(matches!(evs.last(), Some(Event::DelPlayer(3))));
    s.close();
    h.join().unwrap();
}

/// Signature, then ADDPLAYER, the same id with new props, DELPLAYER,
/// ADDPLAYER again, then OTHERPLPROPS with a new nickname. The resend is
/// what the fork does when props change.
fn fake_gserver_resend() -> (String, thread::JoinHandle<()>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap().to_string();
    let h = thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        s.set_read_timeout(Some(TIMEOUT)).unwrap();
        let login = zlib_decompress(&read_raw_frame(&mut s)).unwrap();
        let mut codec = Codec::gen5(login[1] - 0x20);
        send_msgs(&mut s, &mut codec, &[message(25, &[gchar(73)])]);
        let gap = Duration::from_millis(40);
        thread::sleep(gap);
        send_msgs(
            &mut s,
            &mut codec,
            &[addplayer(6, "test205", "onlinestartlocal.nw", "default")],
        );
        thread::sleep(gap);
        send_msgs(
            &mut s,
            &mut codec,
            &[addplayer(6, "test205", "cave.nw", "Ally")],
        );
        thread::sleep(gap);
        send_msgs(&mut s, &mut codec, &[message(56, &[gchar(0), gchar(6)])]);
        thread::sleep(gap);
        send_msgs(
            &mut s,
            &mut codec,
            &[addplayer(6, "test205", "onlinestartlocal.nw", "default")],
        );
        thread::sleep(gap);
        // OTHERPLPROPS (8): id 6, nickname "Ally" only. Whether the fork
        // sends this to an RC is **check**; the RC must apply it if it does.
        let mut p = vec![gchar(0), gchar(6), gchar(0)];
        push_gstring(&mut p, b"Ally");
        send_msgs(&mut s, &mut codec, &[message(8, &p)]);
        let mut rest = Vec::new();
        let _ = s.read_to_end(&mut rest);
    });
    (addr, h)
}

#[test]
fn repeated_addplayer_is_an_update() {
    let (addr, h) = fake_gserver_resend();
    let mut s = Session::login(&addr, &params(RcKind::Gen5), TIMEOUT).unwrap();
    let evs = s
        .settle(Duration::from_millis(300), Instant::now() + TIMEOUT)
        .unwrap();
    let got: Vec<(&str, &str, &str)> = evs
        .iter()
        .filter_map(|ev| match ev {
            Event::AddPlayer(e) => Some(("online", e.level.as_str(), e.nickname.as_str())),
            Event::UpdatePlayer(e) => Some(("update", e.level.as_str(), e.nickname.as_str())),
            Event::DelPlayer(6) => Some(("offline", "", "")),
            _ => None,
        })
        .collect();
    assert_eq!(
        got,
        [
            ("online", "onlinestartlocal.nw", "default"),
            ("update", "cave.nw", "Ally"),
            ("offline", "", ""),
            ("online", "onlinestartlocal.nw", "default"),
            ("update", "onlinestartlocal.nw", "Ally"),
        ]
    );
    assert_eq!(s.players.players[&6].level, "onlinestartlocal.nw");
    assert_eq!(s.players.players[&6].nickname, "Ally");
    s.close();
    h.join().unwrap();
}

type ListSeen = (Vec<u8>, Vec<Message>);

/// A fake listserver answering the RC list the way the lab listserver does.
fn fake_listserver(refuse: bool) -> (String, thread::JoinHandle<ListSeen>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap().to_string();
    let h = thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        s.set_read_timeout(Some(TIMEOUT)).unwrap();
        let mut hs = zlib_decompress(&read_raw_frame(&mut s)).unwrap();
        // Id 5 (gen 5, the 2004 RC): the login follows in its own frame.
        // Id 0 (plain zlib, the 2002/05/02 RC): the login is in the same
        // frame, as rc141 sends it; a lone id 0 is answered as a login by
        // the real listserver, so it is refused here.
        let (mut codec, account) = match hs[0] - 0x20 {
            5 => {
                let mut codec = Codec::gen5(hs[1] - 0x20);
                let account = split_messages(&codec.decode(&read_raw_frame(&mut s)).unwrap());
                (codec, account)
            }
            0 => {
                let end = hs.iter().position(|b| *b == b'\n').unwrap() + 1;
                let account = split_messages(&hs[end..]);
                assert!(!account.is_empty(), "classic login sent in a second frame");
                hs.truncate(end);
                (Codec::Plain, account)
            }
            id => panic!("unexpected RC list handshake id {id}"),
        };
        if refuse {
            send_msgs(
                &mut s,
                &mut codec,
                &[message(4, b"Account does not exist.")],
            );
            return (hs, account);
        }
        let mut list = vec![gchar(0), gchar(2)];
        for (name, port) in [("H My Server", "14802"), ("Lab Classic", "14803")] {
            list.push(gchar(8));
            for f in [
                name,
                "English",
                "lab",
                "",
                "4.0.0-beta",
                "0",
                "127.0.0.1",
                port,
            ] {
                push_gstring(&mut list, f.as_bytes());
            }
        }
        send_msgs(&mut s, &mut codec, &[list]);
        send_msgs(
            &mut s,
            &mut codec,
            &[
                message(2, b"Welcome.\rTwo servers."),
                message(3, b""),
                message(5, b""),
            ],
        );
        (hs, account)
    });
    (addr, h)
}

#[test]
fn rc_list_from_listserver() {
    let (addr, h) = fake_listserver(false);
    let reply = fetch_rc_list(&addr, false, "labrc", "labpw", 0x11, TIMEOUT).unwrap();
    assert_eq!(reply.servers.len(), 2);
    assert_eq!(reply.servers[0].kind, "H");
    assert_eq!(reply.servers[0].name, "My Server");
    assert_eq!(reply.status.as_deref(), Some("Welcome.\nTwo servers."));
    let s = pick(&reply.servers, "classic").unwrap();
    assert_eq!((s.host.as_str(), s.port), ("127.0.0.1", 14803));
    let (hs, account) = h.join().unwrap();
    // The 2004 RC's handshake (lab 2026-11-13): key, then `rc`.
    assert_eq!(&hs[1..], b"1rc\n");
    assert_eq!(account.len(), 1);
    assert_eq!(account[0].id, 1);
    assert_eq!(account[0].payload, b"%labrc%labpw");
}

#[test]
fn rc_list_from_listserver_classic() {
    let (addr, h) = fake_listserver(false);
    let reply = fetch_rc_list(&addr, true, "labrc", "labpw", 0x11, TIMEOUT).unwrap();
    assert_eq!(reply.servers.len(), 2);
    let (hs, account) = h.join().unwrap();
    // The 2002/05/02 RC's handshake: id 0, then `rc`, and the login in the
    // same plain zlib frame.
    assert_eq!(hs, b" rc\n");
    assert_eq!(account[0].payload, b"%labrc%labpw");
}

struct SecureSeen {
    c1: Vec<u8>,
    /// C2's line, when one arrived. `None` when the client sent nothing
    /// after the hello.
    c2: Option<Vec<u8>>,
    opened: Option<Vec<u8>>,
}

/// A fake listserver for the secure RC list. `refuse` sends id 4 instead
/// of S1. Otherwise it answers C1, opens C2, and sends the same list as
/// [`fake_listserver`].
fn fake_secure_listserver(refuse: bool) -> (String, [u8; 32], thread::JoinHandle<SecureSeen>) {
    let identity = ServerIdentity::generate();
    let public = identity.public_key();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(TIMEOUT)).unwrap();
        let c1 = zlib_decompress(&read_raw_frame(&mut stream)).unwrap();
        let line = c1.split(|b| *b == b'\n').next().unwrap_or(&c1);
        if refuse {
            let mut codec = Codec::Plain;
            send_msgs(
                &mut stream,
                &mut codec,
                &[message(4, b"Account does not exist.")],
            );
            return SecureSeen {
                c1,
                c2: None,
                opened: None,
            };
        }
        let hello = ClientHello::parse(line, 2).unwrap().unwrap();
        assert_eq!(
            hello.prefix[0].wrapping_sub(0x20),
            5,
            "RC list id, not id 7"
        );
        assert_eq!(hello.inner_token(), "GSERV025");
        assert!(hello.suffix.is_empty(), "an RC list sends no list name");
        let (s1, opener) = hello.respond(&identity).unwrap();
        let mut codec = Codec::Plain;
        let mut body = s1;
        body.push(b'\n');
        stream.write_all(&codec.encode(&body).unwrap()).unwrap();
        stream.flush().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(800)))
            .unwrap();
        let c2 = match read_raw_frame_timeout(&mut stream) {
            Some(frame) => Some(zlib_decompress(&frame).unwrap()),
            None => {
                return SecureSeen {
                    c1,
                    c2: None,
                    opened: None,
                };
            }
        };
        let c2_line = c2
            .as_deref()
            .and_then(|b| b.split(|byte| *byte == b'\n').next())
            .unwrap_or_default();
        let opened = opener.open(c2_line).ok();
        let key = hello.prefix[1].wrapping_sub(0x20);
        let mut gen5 = Codec::gen5(key);
        let mut list = vec![gchar(0), gchar(1)];
        list.push(gchar(8));
        for field in [
            "H My Server",
            "English",
            "lab",
            "",
            "4.0.0-beta",
            "0",
            "127.0.0.1",
            "14802",
        ] {
            push_gstring(&mut list, field.as_bytes());
        }
        send_msgs(&mut stream, &mut gen5, &[list]);
        send_msgs(
            &mut stream,
            &mut gen5,
            &[message(2, b"Welcome."), message(3, b""), message(5, b"")],
        );
        SecureSeen { c1, c2, opened }
    });
    (addr, public, handle)
}

fn read_raw_frame_timeout(stream: &mut TcpStream) -> Option<Vec<u8>> {
    let mut len = [0u8; 2];
    if stream.read_exact(&mut len).is_err() {
        return None;
    }
    let mut frame = vec![0u8; usize::from(u16::from_be_bytes(len))];
    stream.read_exact(&mut frame).ok()?;
    Some(frame)
}

#[test]
fn secure_rc_list_seals_the_account_and_reads_id_5() {
    let (addr, public, handle) = fake_secure_listserver(false);
    let reply =
        fetch_rc_list_secure(&addr, "GSERV025", "labrc", "labpw", 0x11, &public, TIMEOUT).unwrap();
    assert_eq!(reply.servers.len(), 1);
    assert_eq!(reply.servers[0].kind, "H");
    assert_eq!(reply.servers[0].name, "My Server");
    assert_eq!(reply.status.as_deref(), Some("Welcome."));
    let seen = handle.join().unwrap();
    let c1 = seen.c1;
    // id 5, key 0x11, SEI00001, the RC token, 43-byte key, line feed.
    // No list name and no account: a longer line would be the player-list
    // form or the password in the clear.
    assert_eq!(c1.len(), 2 + 8 + 8 + 43 + 1, "{c1:?}");
    assert_eq!(&c1[..2], &[gchar(5), gchar(0x11)]);
    assert_eq!(&c1[2..18], b"SEI00001GSERV025");
    assert_eq!(*c1.last().unwrap(), b'\n');
    let c2 = seen.c2.expect("C2 was sent");
    assert!(c2.starts_with(seclogin::TOKEN.as_bytes()), "{c2:?}");
    let mut tail = Vec::new();
    push_gstring(&mut tail, b"labrc");
    push_gstring(&mut tail, b"labpw");
    assert_eq!(seen.opened.unwrap(), tail);
}

#[test]
fn secure_rc_list_does_not_send_the_password_for_the_wrong_key() {
    let (addr, _public, handle) = fake_secure_listserver(false);
    let other = ServerIdentity::generate().public_key();
    match fetch_rc_list_secure(&addr, "GSERV025", "labrc", "labpw", 0x11, &other, TIMEOUT) {
        Err(ListError::Secure(why)) => {
            assert!(why.contains("not sending the password"), "{why}");
        }
        other => panic!("wrong result: {other:?}"),
    }
    let seen = handle.join().unwrap();
    assert!(
        seen.c2.is_none(),
        "C2 must not be sent when the key does not match"
    );
}

#[test]
fn secure_rc_list_reports_a_refusal() {
    let (addr, public, handle) = fake_secure_listserver(true);
    match fetch_rc_list_secure(&addr, "GSERV025", "nobody", "x", 0x11, &public, TIMEOUT) {
        Err(ListError::Refused(why)) => assert_eq!(why, "Account does not exist."),
        other => panic!("wrong result: {other:?}"),
    }
    handle.join().unwrap();
}

#[test]
fn servers_command_uses_the_secure_handshake() {
    let (addr, public, handle) = fake_secure_listserver(false);
    let output = Command::new(env!("CARGO_BIN_EXE_seihai-rc"))
        .args([
            "servers",
            "--listserver",
            &addr,
            "--account",
            "labrc",
            "--secure-login",
            "--server-key",
            &seclogin::encode_key(&public),
            "--timeout",
            "5",
        ])
        .env("SEIHAI_RC_PASSWORD", "labpw")
        .env("SEIHAI_RC_TEST_NEW_PW", "labpw2")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("My Server"), "{stdout}");
    let seen = handle.join().unwrap();
    assert!(seen.c2.is_some());
}

#[test]
fn rc_list_refused() {
    let (addr, h) = fake_listserver(true);
    match fetch_rc_list(&addr, false, "nobody", "x", 0x11, TIMEOUT) {
        Err(ListError::Refused(why)) => assert_eq!(why, "Account does not exist."),
        other => panic!("wrong result: {other:?}"),
    }
    h.join().unwrap();
}

// Stage 2 player actions, through the `players` commands. The fake sends
// the list, then records every message the RC sends. With `answer_kick`
// it answers id 61 with DELPLAYER (56) for that id, the way a server that
// drops the player would tell its RCs.

/// The id 72 from the 2026-11-04 capture, after its player id and account.
const CAPTURED_72_AFTER_ACCOUNT: [u8; 177] = {
    const HEX: &[u8] = b"246d61696eb020284c616232323272632123222623202020242a25252621283f2a73776f7264312e706e67292b2b736869656c64312e706e672a2469646c652b8d68656164302e706e672d22202a24322f5c305d323434336f6e6c696e6573746172746c6f63616c2e6e773a203b2020203c2020203d20237b3e202820209f4054422574657374314328626f64792e706e67444e927e202020202324626f6d6223626f772c2d67725f6d6f76656d656e74";
    const fn nib(c: u8) -> u8 {
        if c <= b'9' {
            c - b'0'
        } else {
            c - b'a' + 10
        }
    }
    let mut out = [0u8; 177];
    let mut i = 0;
    while i < 177 {
        out[i] = (nib(HEX[2 * i]) << 4) | nib(HEX[2 * i + 1]);
        i += 1;
    }
    out
};

fn fake_gserver_actions(answer_kick: bool) -> (String, thread::JoinHandle<Vec<Message>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap().to_string();
    let h = thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        s.set_read_timeout(Some(TIMEOUT)).unwrap();
        let login = zlib_decompress(&read_raw_frame(&mut s)).unwrap();
        let mut codec = match login[0] - 0x20 {
            6 => Codec::gen5(login[1] - 0x20),
            1 => Codec::Plain,
            t => panic!("unexpected login type {t}"),
        };
        send_msgs(
            &mut s,
            &mut codec,
            &[
                message(25, &[gchar(73)]),
                addplayer(2, "alice", "start.nw", "Alice"),
                addplayer(3, "bob", "cave.nw", "Bob"),
                addplayer(200, "staff", " ", "*staff"),
            ],
        );
        let mut seen = Vec::new();
        // Per-account comments and ban, kept as the bytes the RC sent.
        let mut comments: Vec<u8> = Vec::new();
        let mut ban: Vec<u8> = vec![gchar(0)];
        // Rights 0, IP range 0.0.0.0, folders as the fork returns them:
        // untokenized.
        let mut rights: Vec<u8> = b"     '0.0.0.0 +rw levels/*".to_vec();
        let reply = |id: u8, account: &[u8], rest: &[u8]| {
            let mut p = Vec::new();
            push_gstring(&mut p, account);
            p.extend_from_slice(rest);
            message(id, &p)
        };
        let mut props = CAPTURED_72_AFTER_ACCOUNT.to_vec();
        // Accounts by name, each as the 73 that answers a get (capture 3).
        let mut accounts: Vec<(Vec<u8>, Vec<u8>)> = vec![(
            b"test1".to_vec(),
            b"%test1 3lab@example.invalid   $main  ".to_vec(),
        )];
        // A 78 or 70 payload as the 73 the server would send: no password,
        // and one 0x20 before the reason.
        let as_73 = |p: &[u8]| -> (Vec<u8>, Vec<u8>) {
            let mut i = 0;
            let gs = |i: &mut usize| {
                let n = usize::from(p[*i] - 0x20);
                let v = p[*i + 1..*i + 1 + n].to_vec();
                *i += 1 + n;
                v
            };
            let account = gs(&mut i);
            gs(&mut i);
            let email = gs(&mut i);
            let flags = p[i..i + 3].to_vec();
            i += 3;
            let worlds = gs(&mut i);
            let reason = gs(&mut i);
            let mut r = Vec::new();
            push_gstring(&mut r, &account);
            push_gstring(&mut r, b"");
            push_gstring(&mut r, &email);
            r.extend_from_slice(&flags);
            push_gstring(&mut r, &worlds);
            r.push(0x20);
            push_gstring(&mut r, &reason);
            (account, r)
        };
        let after_account = |p: &[u8]| -> (Vec<u8>, Vec<u8>) {
            let n = usize::from(p[0] - 0x20);
            (p[1..1 + n].to_vec(), p[1 + n..].to_vec())
        };
        while let Some(frame) = read_raw_frame_timeout(&mut s) {
            for m in split_messages(&codec.decode(&frame).unwrap()) {
                if m.id == 2 {
                    // The nickname after the login; checked in the rcs tests.
                    continue;
                }
                match m.id {
                    61 if answer_kick => {
                        send_msgs(&mut s, &mut codec, &[message(56, &m.payload)]);
                    }
                    85 => {
                        let r = reply(63, &m.payload, &comments);
                        send_msgs(&mut s, &mut codec, &[r]);
                    }
                    86 => comments = after_account(&m.payload).1,
                    87 => {
                        let r = reply(64, &m.payload, &ban);
                        send_msgs(&mut s, &mut codec, &[r]);
                    }
                    88 => ban = after_account(&m.payload).1,
                    83 => {
                        let r = reply(62, &m.payload, &rights);
                        send_msgs(&mut s, &mut codec, &[r]);
                    }
                    84 => rights = after_account(&m.payload).1,
                    73 => {
                        // The captured 72 (or what a 76 set), for the
                        // player asked about.
                        let mut r = m.payload.clone();
                        push_gstring(&mut r, b"bob");
                        r.extend_from_slice(&props);
                        send_msgs(&mut s, &mut codec, &[message(72, &r)]);
                    }
                    74 => {
                        // By account: player id 0, as the fork sent it.
                        let (account, _) = after_account(&m.payload);
                        let mut r = vec![0x20, 0x20];
                        push_gstring(&mut r, &account);
                        r.extend_from_slice(&props);
                        send_msgs(&mut s, &mut codec, &[message(72, &r)]);
                    }
                    77 => {
                        if let Some((_, r)) = accounts.iter().find(|(n, _)| *n == m.payload) {
                            send_msgs(&mut s, &mut codec, &[message(73, r)]);
                        }
                    }
                    70 | 78 => {
                        let (name, r) = as_73(&m.payload);
                        accounts.retain(|(n, _)| *n != name);
                        accounts.push((name, r));
                    }
                    71 => accounts.retain(|(n, _)| *n != m.payload),
                    72 => {
                        // The pattern: `%` alone, a prefix then `%`, or a name.
                        let (pattern, _) = after_account(&m.payload);
                        let mut r = Vec::new();
                        for (n, _) in &accounts {
                            let hit = match pattern.strip_suffix(b"%") {
                                Some(prefix) => n.starts_with(prefix),
                                None => *n == pattern,
                            };
                            if hit {
                                push_gstring(&mut r, n);
                            }
                        }
                        send_msgs(&mut s, &mut codec, &[message(70, &r)]);
                    }
                    76 => {
                        // Everything after the account is the 72 tail, then
                        // one GCHAR 0.
                        let rest = after_account(&m.payload).1;
                        assert_eq!(rest.last(), Some(&0x20));
                        props = rest[..rest.len() - 1].to_vec();
                    }
                    _ => {}
                }
                seen.push(m);
            }
        }
        seen
    });
    (addr, h)
}

fn run_players(addr: &str, extra: &[&str]) -> (bool, String, String) {
    run_group("players", addr, extra)
}

fn run_group(group: &str, addr: &str, extra: &[&str]) -> (bool, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_seihai-rc"))
        .arg(group)
        .args(extra)
        .args([
            "--server",
            addr,
            "--account",
            "labrc",
            "--settle",
            "200",
            "--timeout",
            "5",
        ])
        .env("SEIHAI_RC_PASSWORD", "labpw")
        .env("SEIHAI_RC_TEST_NEW_PW", "labpw2")
        .output()
        .unwrap();
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn rcs_list_prints_only_the_rcs_from_the_player_list() {
    let (addr, h) = fake_gserver(true);
    let (ok, out, err) = run_group("rcs", &addr, &["list"]);
    assert!(ok, "{err}");
    assert!(out.contains("200  staff"), "{out}");
    assert!(out.contains("*staff"), "{out}");
    assert!(!out.contains("alice"), "a player is not an RC: {out}");
    assert!(out.contains("1 other RC(s)"), "{out}");
    let seen = h.join().unwrap();
    // Built from ADDPLAYER/DELPLAYER only: id 65 is never sent.
    assert!(seen.after.iter().all(|m| m.id != 65), "{:?}", seen.after);
    // With no --nick the CLI sends `*account`, as real RCs always send one.
    assert_eq!(seen.after[0].id, 2);
    assert_eq!(seen.after[0].payload, b" &*labrc");
}

#[test]
fn no_nick_sends_no_nickname() {
    let (addr, h) = fake_gserver(true);
    let (ok, _, err) = run_group("rcs", &addr, &["list", "--no-nick"]);
    assert!(ok, "{err}");
    let seen = h.join().unwrap();
    assert!(seen.after.iter().all(|m| m.id != 2), "{:?}", seen.after);
}

#[test]
fn players_list_json() {
    let (addr, h) = fake_gserver(true);
    let (ok, out, err) = run_group("players", &addr, &["list", "--json"]);
    assert!(ok, "{err}");
    assert_eq!(
        out.trim(),
        concat!(
            "[{\"id\":2,\"kind\":\"player\",\"account\":\"alice\",\"nickname\":\"Alice\",",
            "\"level\":\"start.nw\",\"status\":0,\"community\":\"alice\"},",
            "{\"id\":200,\"kind\":\"rc\",\"account\":\"staff\",\"nickname\":\"*staff\",",
            "\"level\":\" \",\"status\":0,\"community\":\"staff\"}]"
        )
    );
    h.join().unwrap();
}

#[test]
fn json_and_tsv_together_are_refused() {
    let (ok, _, err) = run_group("players", "127.0.0.1:1", &["list", "--json", "--tsv"]);
    assert!(!ok);
    assert!(err.contains("--json or --tsv"), "{err}");
}

#[test]
fn identity_is_sent_after_the_password() {
    let (addr, h) = fake_gserver(true);
    let (ok, _, err) = run_group("rcs", &addr, &["list", "--identity", "win,lab"]);
    assert!(ok, "{err}");
    let seen = h.join().unwrap();
    assert!(
        seen.login.ends_with(b"%labrc%labpwwin,lab\n"),
        "{:?}",
        seen.login
    );
}

#[test]
fn rcs_list_tsv() {
    let (addr, h) = fake_gserver(true);
    let (ok, out, err) = run_group("rcs", &addr, &["list", "--tsv"]);
    assert!(ok, "{err}");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "id\taccount\tnickname\tstatus\tcommunity");
    assert_eq!(lines.len(), 2, "{out}");
    assert!(lines[1].starts_with("200\tstaff\t*staff\t"), "{out}");
    h.join().unwrap();
}

#[test]
fn players_pm_sends_count_ids_and_text() {
    let (addr, h) = fake_gserver_actions(false);
    let (ok, out, err) = run_players(&addr, &["pm", "2,bob", "hello", "there"]);
    assert!(ok, "{err}");
    assert!(out.contains("sent private message to 2,3"), "{out}");
    let seen = h.join().unwrap();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].id, 28);
    assert_eq!(seen[0].payload, b" \" \" #\"hello there\"");
}

#[test]
fn players_pm_classic_login() {
    let (addr, h) = fake_gserver_actions(false);
    let (ok, _, err) = run_players(&addr, &["pm", "staff", "hi", "--rc", "classic"]);
    assert!(ok, "{err}");
    let seen = h.join().unwrap();
    assert_eq!(seen[0].id, 28);
    // count 1, id 200 = (1 << 7) | 72.
    assert_eq!(seen[0].payload, b" !!h\"hi\"");
}

#[test]
fn players_say_sends_the_admin_message() {
    let (addr, h) = fake_gserver_actions(false);
    let (ok, out, err) = run_players(&addr, &["say", "Restart", "in", "5"]);
    assert!(ok, "{err}");
    assert!(out.contains("sent admin message"), "{out}");
    let seen = h.join().unwrap();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].id, 63);
    assert_eq!(seen[0].payload, b"Restart in 5");
}

#[test]
fn players_admin_pm_sends_the_id_then_text() {
    let (addr, h) = fake_gserver_actions(false);
    let (ok, out, err) = run_players(&addr, &["admin-pm", "bob", "Please", "stop"]);
    assert!(ok, "{err}");
    assert!(out.contains("sent admin message to 3 bob"), "{out}");
    let seen = h.join().unwrap();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].id, 64);
    assert_eq!(seen[0].payload, b" #Please stop");
}

#[test]
fn players_kick_waits_for_delplayer() {
    let (addr, h) = fake_gserver_actions(true);
    let (ok, out, err) = run_players(&addr, &["kick", "bob"]);
    assert!(ok, "{err}");
    assert!(out.contains("disconnected 3 bob"), "{out}");
    let seen = h.join().unwrap();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].id, 61);
    assert_eq!(seen[0].payload, [0x20, 0x23]);
}

#[test]
fn players_kick_fails_when_the_player_stays() {
    let (addr, h) = fake_gserver_actions(false);
    let (ok, _, err) = run_players(&addr, &["kick", "3"]);
    assert!(!ok);
    assert!(err.contains("no DELPLAYER"), "{err}");
    assert_eq!(h.join().unwrap()[0].id, 61);
}

#[test]
fn players_warp_sends_id_half_tiles_and_level() {
    let (addr, h) = fake_gserver_actions(false);
    let (ok, out, err) = run_players(&addr, &["warp", "alice", "cave.nw", "30.5", "12"]);
    assert!(ok, "{err}");
    assert!(out.contains("sent warp for 2 alice"), "{out}");
    let seen = h.join().unwrap();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].id, 82);
    let mut want = vec![0x20, 0x22, 0x20 + 61, 0x20 + 24];
    want.extend_from_slice(b"cave.nw");
    assert_eq!(seen[0].payload, want);
}

#[test]
fn players_actions_refuse_who_is_not_listed() {
    for args in [
        &["warp", "carol", "cave.nw", "1", "1"][..],
        &["warp", "staff", "cave.nw", "1", "1"][..],
        &["kick", "9"][..],
        &["pm", "2,carol", "hi"][..],
        &["admin-pm", "carol", "hi"][..],
    ] {
        let (addr, h) = fake_gserver_actions(true);
        let (ok, _, err) = run_players(&addr, args);
        assert!(!ok, "{args:?} should fail");
        assert!(
            err.contains("no player or RC") || err.contains("is an RC"),
            "{args:?}: {err}"
        );
        assert!(h.join().unwrap().is_empty(), "{args:?} sent something");
    }
}

#[test]
fn players_comments_get_and_set() {
    let (addr, h) = fake_gserver_actions(false);
    let (ok, out, err) = run_players(&addr, &["comments", "test1"]);
    assert!(ok, "{err}");
    assert_eq!(out, "");
    let seen = h.join().unwrap();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].id, 85);
    assert_eq!(seen[0].payload, b"test1");

    let dir = std::env::temp_dir().join(format!("seihai-rc-comments-{}", std::process::id()));
    std::fs::write(&dir, "lab comment one\nline two\n").unwrap();
    let (addr, h) = fake_gserver_actions(false);
    let (ok, out, err) = run_players(
        &addr,
        &["comments", "test1", "--set-file", dir.to_str().unwrap()],
    );
    let _ = std::fs::remove_file(&dir);
    assert!(ok, "{err}");
    assert_eq!(out, "lab comment one\nline two\n");
    let seen = h.join().unwrap();
    assert_eq!(seen[0].id, 86);
    // The bytes the real RC sent in the 2026-11-04 capture.
    assert_eq!(seen[0].payload, b"%test1\"lab comment one\",\"line two\"");
    assert_eq!(seen[1].id, 85);
}

#[test]
fn players_ban_get_and_set() {
    let (addr, h) = fake_gserver_actions(false);
    let (ok, out, err) = run_players(
        &addr,
        &["ban", "test1", "--set", "on", "--reason", "lab ban reason"],
    );
    assert!(ok, "{err}");
    assert_eq!(out, "test1: banned\nreason: lab ban reason\n");
    let seen = h.join().unwrap();
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert_eq!(seen[0].id, 88);
    assert_eq!(seen[0].payload, b"%test1!\"lab ban reason\"");
    assert_eq!(seen[1].id, 87);
    assert_eq!(seen[1].payload, b"test1");

    let (addr, h) = fake_gserver_actions(false);
    let (ok, out, err) = run_players(&addr, &["ban", "test1"]);
    assert!(ok, "{err}");
    assert_eq!(out, "test1: not banned\n");
    assert_eq!(h.join().unwrap().len(), 1);
}

#[test]
fn players_ban_without_a_reply_fails() {
    // A server that never answers id 87: the fork after a ban set on an
    // online player (2026-11-04 lab) stopped sending anything.
    let (addr, h) = fake_gserver(true);
    let output = Command::new(env!("CARGO_BIN_EXE_seihai-rc"))
        .args([
            "players",
            "ban",
            "test1",
            "--server",
            &addr,
            "--account",
            "labrc",
            "--timeout",
            "1",
        ])
        .env("SEIHAI_RC_PASSWORD", "labpw")
        .env("SEIHAI_RC_TEST_NEW_PW", "labpw2")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(err.contains("no reply (id 64)"), "{err}");
    h.join().unwrap();
}

#[test]
fn players_rights_reads_the_forks_untokenized_folders_as_one_line() {
    let (addr, h) = fake_gserver_actions(false);
    let (ok, out, err) = run_players(&addr, &["rights", "test1"]);
    assert!(ok, "{err}");
    assert_eq!(
        out,
        "account: test1\nrights: 0 (bits: none)\nip range: 0.0.0.0\nfolder: rw levels/*\n"
    );
    let seen = h.join().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].id, 83);
    assert_eq!(seen[0].payload, b"test1");
}

#[test]
fn players_rights_set_keeps_the_other_fields() {
    let (addr, h) = fake_gserver_actions(false);
    let (ok, out, err) = run_players(&addr, &["rights", "test1", "--set", "513"]);
    assert!(ok, "{err}");
    assert!(out.contains("rights: 513 (bits: 0, 9)"), "{out}");
    assert!(out.contains("folder: rw levels/*"), "{out}");
    let seen = h.join().unwrap();
    let ids: Vec<u8> = seen.iter().map(|m| m.id).collect();
    assert_eq!(ids, [83, 84, 83]);
    // The bytes the real RC sent in the 2026-11-04 capture: the folder
    // line goes back tokenized even though the fork sent it bare.
    assert_eq!(seen[1].payload, b"%test1   $!'0.0.0.0 -\"rw levels/*\"");
}

#[test]
fn players_props_prints_the_captured_reply() {
    let (addr, h) = fake_gserver_actions(false);
    let (ok, out, err) = run_players(&addr, &["props", "bob"]);
    assert!(ok, "{err}");
    assert!(
        out.starts_with("player 3 account bob world main\n"),
        "{out}"
    );
    assert!(out.contains("   3  gralats               0\n"), "{out}");
    assert!(out.contains("  29  online seconds        475\n"), "{out}");
    assert!(out.contains("weapons: bomb, bow, -gr_movement"), "{out}");
    let seen = h.join().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].id, 73);
    assert_eq!(seen[0].payload, b" #");
}

#[test]
fn players_props_reads_an_offline_account_by_name() {
    let (addr, h) = fake_gserver_actions(false);
    let (ok, out, err) = run_players(&addr, &["props", "carol"]);
    assert!(ok, "{err}");
    assert!(
        out.starts_with("account carol world main (not online)\n"),
        "{out}"
    );
    assert!(out.contains("flags: 0\n"), "{out}");
    let seen = h.join().unwrap();
    assert_eq!(seen.len(), 1);
    // The bytes the real RC sent for an offline account (capture 2).
    assert_eq!(seen[0].id, 74);
    assert_eq!(seen[0].payload, b"%carol ");
}

#[test]
fn players_props_set_sends_76_and_reads_back() {
    let (addr, h) = fake_gserver_actions(false);
    let (ok, out, err) = run_players(&addr, &["props", "bob", "--set", "gralats=7,bombs=9"]);
    assert!(ok, "{err}");
    assert!(err.contains("set the props for bob"), "{err}");
    assert!(out.contains("   3  gralats               7\n"), "{out}");
    assert!(out.contains("   5  bombs                 9\n"), "{out}");
    // Online seconds go back as 0, as the real RC sends them.
    assert!(out.contains("  29  online seconds        0\n"), "{out}");
    let seen = h.join().unwrap();
    let ids: Vec<u8> = seen.iter().map(|m| m.id).collect();
    assert_eq!(ids, [73, 76, 73]);
    assert!(seen[1].payload.starts_with(b"#bob$main"));
}

#[test]
fn players_props_set_changes_flags_weapons_and_images() {
    let (addr, h) = fake_gserver_actions(false);
    let (ok, out, err) = run_players(
        &addr,
        &[
            "props",
            "bob",
            "--set",
            "sword=3,head=head5.png,level=cave.nw,kills=300",
            "--add-flag",
            "quest=2",
            "--add-weapon",
            "lantern",
            "--remove-weapon",
            "bow",
        ],
    );
    assert!(ok, "{err}");
    assert!(err.contains("set the props for bob"), "{err}");
    assert!(
        out.contains("   8  sword                 3 sword1.png\n"),
        "{out}"
    );
    assert!(
        out.contains("  11  head                  head5.png\n"),
        "{out}"
    );
    assert!(
        out.contains("  20  level                 cave.nw\n"),
        "{out}"
    );
    assert!(out.contains("  27  kills                 300\n"), "{out}");
    assert!(out.contains("flags: 1\n  quest=2\n"), "{out}");
    assert!(
        out.contains("weapons: bomb, -gr_movement, lantern\n"),
        "{out}"
    );
    let seen = h.join().unwrap();
    let ids: Vec<u8> = seen.iter().map(|m| m.id).collect();
    assert_eq!(ids, [73, 76, 73]);
}

#[test]
fn players_props_refuses_an_empty_flag_before_logging_in() {
    let (ok, _, err) = run_players("127.0.0.1:1", &["props", "bob", "--add-flag", "=1"]);
    assert!(!ok);
    assert!(err.contains("--add-flag needs"), "{err}");
}

#[test]
fn players_props_set_refuses_an_unknown_name_before_logging_in() {
    let (ok, _, err) = run_players("127.0.0.1:1", &["props", "bob", "--set", "nick=x"]);
    assert!(!ok);
    assert!(err.contains("not a prop this client sets"), "{err}");
}

#[test]
fn accounts_list_get_and_set_follow_capture_3() {
    let (addr, h) = fake_gserver_actions(false);
    let (ok, out, err) = run_group(
        "accounts",
        &addr,
        &[
            "set",
            "test1",
            "--admin-level",
            "2",
            "--reason",
            "lab reason",
        ],
    );
    assert!(ok, "{err}");
    assert!(out.contains("admin level: 2\n"), "{out}");
    assert!(out.contains("ban reason: lab reason\n"), "{out}");
    assert!(out.contains("e-mail: lab@example.invalid\n"), "{out}");
    let seen = h.join().unwrap();
    let ids: Vec<u8> = seen.iter().map(|m| m.id).collect();
    assert_eq!(ids, [77, 78, 77]);
    assert_eq!(seen[0].payload, b"test1");
    // The rest goes back as read; no ban time, the password left empty.
    assert_eq!(
        seen[1].payload,
        b"%test1 3lab@example.invalid  \"$main,\"lab reason\""
    );

    let (addr, h) = fake_gserver_actions(false);
    let (ok, out, err) = run_group("accounts", &addr, &["list"]);
    assert!(ok, "{err}");
    assert!(out.ends_with("test1\n"), "{out}");
    let seen = h.join().unwrap();
    assert_eq!(seen[0].id, 72);
    assert_eq!(seen[0].payload, b"!% ");
}

#[test]
fn accounts_add_then_delete() {
    let (addr, h) = fake_gserver_actions(false);
    let (ok, out, err) = run_group(
        "accounts",
        &addr,
        &[
            "add",
            "labadd",
            "--new-password-env",
            "SEIHAI_RC_TEST_NEW_PW",
            "--email",
            "add@example.invalid",
            "--admin-level",
            "1",
        ],
    );
    assert!(ok, "{err}");
    assert!(out.contains("admin worlds: all\n"), "{out}");
    let seen = h.join().unwrap();
    let ids: Vec<u8> = seen.iter().map(|m| m.id).collect();
    assert_eq!(ids, [70, 77]);
    // The bytes the real RC sent for its add (capture 3), with this
    // test's placeholder password.
    assert_eq!(
        seen[0].payload,
        b"&labadd&labpw23add@example.invalid  !#all "
    );

    let (addr, h) = fake_gserver_actions(false);
    let (ok, out, err) = run_group("accounts", &addr, &["delete", "test1", "--yes"]);
    assert!(ok, "{err}");
    assert_eq!(out.lines().last(), Some("deleted test1"));
    let seen = h.join().unwrap();
    let ids: Vec<u8> = seen.iter().map(|m| m.id).collect();
    assert_eq!(ids, [71, 72]);
    assert_eq!(seen[0].payload, b"test1");
}

#[test]
fn accounts_refuse_before_logging_in() {
    for args in [
        &["delete", "test1"][..],
        &["add", "labadd"][..],
        &["set", "test1"][..],
        &["set", "test1", "--banned", "maybe"][..],
        &["set", "test1", "--admin-level", "300"][..],
    ] {
        let (ok, _, err) = run_group("accounts", "127.0.0.1:1", args);
        assert!(!ok, "{args:?}");
        assert!(!err.contains("127.0.0.1:1"), "{args:?}: {err}");
    }
}
