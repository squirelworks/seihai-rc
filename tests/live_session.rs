//! The window's live session (`seihai_rc::live`) against a fake gserver on
//! an ephemeral loopback port: the worker thread logs in, passes events up,
//! sends commands down, reads props and ends cleanly. No egui and no
//! display, so it runs in CI without the `gui` feature.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use seihai_core::protocol::{split_messages, zlib_decompress, Message};
use seihai_rc::link::Codec;
use seihai_rc::live::{Command, LiveSession, RcView, Status, Update};
use seihai_rc::rc::{Event, LoginParams, RcKind};
use seihai_rc::wire::{gchar, message, push_gstring};

const TIMEOUT: Duration = Duration::from_secs(5);

/// The props block of the 72 captured on 2026-11-04, after the account.
const CAPTURED_72_AFTER_ACCOUNT: &str = "246d61696eb020284c616232323272632123222623202020242a25252621283f2a73776f7264312e706e67292b2b736869656c64312e706e672a2469646c652b8d68656164302e706e672d22202a24322f5c305d323434336f6e6c696e6573746172746c6f63616c2e6e773a203b2020203c2020203d20237b3e202820209f4054422574657374314328626f64792e706e67444e927e202020202324626f6d6223626f772c2d67725f6d6f76656d656e74";

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn read_raw_frame(s: &mut TcpStream) -> Option<Vec<u8>> {
    let mut len = [0u8; 2];
    s.read_exact(&mut len).ok()?;
    let mut f = vec![0u8; usize::from(u16::from_be_bytes(len))];
    s.read_exact(&mut f).ok()?;
    Some(f)
}

fn send_msgs(s: &mut TcpStream, codec: &mut Codec, msgs: &[Vec<u8>]) {
    let mut body = Vec::new();
    for m in msgs {
        body.extend_from_slice(m);
        body.push(b'\n');
    }
    s.write_all(&codec.encode(&body).unwrap()).unwrap();
}

fn addplayer(id: u16, account: &str, level: &str) -> Vec<u8> {
    let mut p = vec![gchar((id >> 7) as u8), gchar((id & 0x7F) as u8)];
    push_gstring(&mut p, account.as_bytes());
    p.push(gchar(20));
    push_gstring(&mut p, level.as_bytes());
    message(55, &p)
}

/// One connection: welcome burst, then answers RC chat (echo), kick
/// (DELPLAYER) and props get (the captured 72), and records what came.
/// With `end_after_kick` it closes the link after answering a kick.
fn fake_gserver(end_after_kick: bool) -> (String, thread::JoinHandle<Vec<Message>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap().to_string();
    let h = thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        s.set_read_timeout(Some(TIMEOUT)).unwrap();
        let login = zlib_decompress(&read_raw_frame(&mut s).unwrap()).unwrap();
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
                addplayer(2, "alice", "start.nw"),
                addplayer(3, "bob", "cave.nw"),
                addplayer(200, "staff", " "),
            ],
        );
        let mut seen = Vec::new();
        while let Some(frame) = read_raw_frame(&mut s) {
            for m in split_messages(&codec.decode(&frame).unwrap()) {
                match m.id {
                    79 => {
                        let mut line = b"labrc: ".to_vec();
                        line.extend_from_slice(&m.payload);
                        send_msgs(&mut s, &mut codec, &[message(74, &line)]);
                    }
                    61 => {
                        send_msgs(&mut s, &mut codec, &[message(56, &m.payload)]);
                        if end_after_kick {
                            seen.push(m);
                            return seen;
                        }
                    }
                    73 => {
                        let mut r = m.payload.clone();
                        push_gstring(&mut r, b"bob");
                        r.extend_from_slice(&unhex(CAPTURED_72_AFTER_ACCOUNT));
                        send_msgs(&mut s, &mut codec, &[message(72, &r)]);
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

fn params(kind: RcKind) -> LoginParams {
    LoginParams {
        kind,
        token: kind.default_token().into(),
        account: "labrc".into(),
        password: "labpw".into(),
        key: 0x21,
        nickname: Some("Lab RC".into()),
        identity: None,
    }
}

/// Apply updates to `view` until `done` holds or the time is up.
fn pump(live: &LiveSession, view: &mut RcView, done: impl Fn(&RcView) -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !done(view) {
        assert!(Instant::now() < deadline, "timed out; log: {:#?}", view.log);
        if let Some(u) = live.recv_timeout(Duration::from_millis(50)) {
            view.apply(u);
        }
    }
}

#[test]
fn live_session_logs_in_lists_players_and_sends_commands() {
    for kind in [RcKind::Gen5, RcKind::Classic] {
        let (addr, h) = fake_gserver(false);
        let live = LiveSession::start(addr.clone(), params(kind), TIMEOUT);
        let mut view = RcView::default();
        pump(&live, &mut view, |v| v.players.players.len() == 3);
        assert_eq!(view.status, Status::Online(addr.clone()));
        let order: Vec<u16> = view.sorted_players().iter().map(|e| e.id).collect();
        assert_eq!(order, vec![2, 3, 200]);

        assert!(live.send(Command::Chat("hello staff".into())));
        pump(&live, &mut view, |v| {
            v.log.iter().any(|l| l == "[rc] labrc: hello staff")
        });

        assert!(live.send(Command::Props(3)));
        pump(&live, &mut view, |v| v.props.is_some());
        let (id, lines) = view.props.clone().unwrap();
        assert_eq!(id, 3);
        assert_eq!(lines[0], "player 3 account bob world main");
        assert!(lines
            .iter()
            .any(|l| l == "weapons: bomb, bow, -gr_movement"));

        assert!(live.send(Command::Pm {
            ids: vec![2],
            text: "hi alice".into()
        }));
        assert!(live.send(Command::AdminAll("restart soon".into())));
        assert!(live.send(Command::AdminPm {
            id: 3,
            text: "stop".into()
        }));
        assert!(live.send(Command::Warp {
            id: 2,
            level: "cave.nw".into(),
            x: 30.5,
            y: 12.0
        }));
        // A bad coordinate is refused before anything is sent.
        assert!(live.send(Command::Warp {
            id: 2,
            level: "cave.nw".into(),
            x: 0.3,
            y: 1.0
        }));
        assert!(live.send(Command::Kick(3)));
        pump(&live, &mut view, |v| !v.players.players.contains_key(&3));
        assert!(view.log.iter().any(|l| l == "[sent] disconnected 3"));
        assert!(view.log.iter().any(|l| l.starts_with("[error] ")));
        assert!(view.log.iter().any(|l| l == "[offline] 3"));

        drop(live); // sends Quit and joins the worker
        let seen = h.join().unwrap();
        let ids: Vec<u8> = seen.iter().map(|m| m.id).filter(|&i| i != 162).collect();
        // Nickname, chat, props get, PM, admin all, admin PM, warp, kick.
        assert_eq!(ids, vec![2, 79, 73, 28, 63, 64, 82, 61], "{kind:?}");
    }
}

#[test]
fn live_session_reports_a_closed_link() {
    let (addr, h) = fake_gserver(true);
    let live = LiveSession::start(addr, params(RcKind::Gen5), TIMEOUT);
    let mut view = RcView::default();
    pump(&live, &mut view, |v| v.players.players.len() == 3);
    live.send(Command::Kick(2));
    pump(&live, &mut view, |v| matches!(v.status, Status::Closed(_)));
    assert_eq!(
        view.status,
        Status::Closed("the server closed the link".into())
    );
    assert!(view.players.players.is_empty());
    h.join().unwrap();
}

#[test]
fn live_session_reports_a_refused_login() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap().to_string();
    let h = thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let login = zlib_decompress(&read_raw_frame(&mut s).unwrap()).unwrap();
        let mut codec = Codec::gen5(login[1] - 0x20);
        send_msgs(
            &mut s,
            &mut codec,
            &[message(16, b"You do not have RC rights.")],
        );
    });
    let live = LiveSession::start(addr, params(RcKind::Gen5), TIMEOUT);
    let u = live.recv_timeout(TIMEOUT).unwrap();
    match &u {
        Update::LoginFailed(why) => assert!(why.contains("RC rights"), "{why}"),
        other => panic!("expected LoginFailed, got {other:?}"),
    }
    let mut view = RcView::default();
    view.apply(u);
    assert!(matches!(view.status, Status::Failed(_)));
    h.join().unwrap();
}

#[test]
fn event_lines_match_the_login_command() {
    use seihai_rc::live::event_line;
    assert_eq!(
        event_line(&Event::AdminMessage("Admin\u{a7}hi".into()), false),
        Some("[admin] Admin: hi".into())
    );
    assert_eq!(
        event_line(
            &Event::PrivateMessage {
                from: 4,
                text: "yo".into()
            },
            false
        ),
        Some("[pm from 4] yo".into())
    );
    assert_eq!(event_line(&Event::Signature, true), None);
}
