//! Login and player list against small fake servers on ephemeral loopback
//! ports. The fakes follow the wire shapes in `docs/PROTOCOL.md`; they check
//! what the RC sends and answer the way the gserver-v2 fork did in the lab.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use seihai_core::protocol::{split_messages, zlib_decompress, Message};
use seihai_rc::link::Codec;
use seihai_rc::listserver::{fetch_rc_list, pick, ListError};
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

type ListSeen = (Vec<u8>, Vec<Message>);

/// A fake listserver answering the RC list the way the lab listserver does.
fn fake_listserver(refuse: bool) -> (String, thread::JoinHandle<ListSeen>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap().to_string();
    let h = thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        s.set_read_timeout(Some(TIMEOUT)).unwrap();
        let hs = zlib_decompress(&read_raw_frame(&mut s)).unwrap();
        assert_eq!(hs[0] - 0x20, 5);
        let mut codec = Codec::gen5(hs[1] - 0x20);
        let account = split_messages(&codec.decode(&read_raw_frame(&mut s)).unwrap());
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
    let reply = fetch_rc_list(&addr, "GSERV025", "labrc", "labpw", 0x11, TIMEOUT).unwrap();
    assert_eq!(reply.servers.len(), 2);
    assert_eq!(reply.servers[0].kind, "H");
    assert_eq!(reply.servers[0].name, "My Server");
    assert_eq!(reply.status.as_deref(), Some("Welcome.\nTwo servers."));
    let s = pick(&reply.servers, "classic").unwrap();
    assert_eq!((s.host.as_str(), s.port), ("127.0.0.1", 14803));
    let (hs, account) = h.join().unwrap();
    assert_eq!(&hs[1..], b"1GSERV025\n");
    assert_eq!(account.len(), 1);
    assert_eq!(account[0].id, 1);
    assert_eq!(account[0].payload, b"%labrc%labpw");
}

#[test]
fn rc_list_refused() {
    let (addr, h) = fake_listserver(true);
    match fetch_rc_list(&addr, "GSERV025", "nobody", "x", 0x11, TIMEOUT) {
        Err(ListError::Refused(why)) => assert_eq!(why, "Account does not exist."),
        other => panic!("wrong result: {other:?}"),
    }
    h.join().unwrap();
}
