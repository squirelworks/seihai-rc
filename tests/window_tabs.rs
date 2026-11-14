//! The window's Accounts, Files and NPC Control tabs, through their
//! session workers (`seihai_rc::live`, `seihai_rc::live_nc`), against fake
//! servers on ephemeral loopback ports. The fakes write their replies from
//! `docs/PROTOCOL.md` themselves; no egui and no display.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use seihai_core::protocol::{read_gint3, split_messages, zlib_decompress, Message};
use seihai_rc::accounts::Account;
use seihai_rc::link::Codec;
use seihai_rc::live::{Command, LiveSession, RcView};
use seihai_rc::live_nc::{LiveNc, NcCommand, NcView, ScriptTarget};
use seihai_rc::rc::{LoginParams, RcKind};
use seihai_rc::wire::{gchar, message, push_gint3, push_gint5, push_gshort, push_gstring, text};

const TIMEOUT: Duration = Duration::from_secs(5);

fn read_frame(s: &mut TcpStream) -> Option<Vec<u8>> {
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

fn addplayer(id: u16, account: &str, level: &str, nick: &str) -> Vec<u8> {
    let mut p = Vec::new();
    push_gshort(&mut p, id);
    push_gstring(&mut p, account.as_bytes());
    p.push(gchar(20));
    push_gstring(&mut p, level.as_bytes());
    p.push(gchar(0));
    push_gstring(&mut p, nick.as_bytes());
    message(55, &p)
}

/// Id 73 for an account.
fn account_reply(a: &Account) -> Vec<u8> {
    let mut p = Vec::new();
    push_gstring(&mut p, a.account.as_bytes());
    push_gstring(&mut p, b"");
    push_gstring(&mut p, a.email.as_bytes());
    p.push(gchar(u8::from(a.banned)));
    p.push(gchar(u8::from(a.load_only)));
    p.push(gchar(a.admin_level));
    push_gstring(&mut p, a.admin_worlds.as_bytes());
    push_gstring(&mut p, a.ban_length.as_bytes());
    push_gstring(&mut p, b"");
    message(73, &p)
}

/// Id 67 `Folder changed to`, then id 66 with the files.
fn dir_reply(folder: &str, names: &[String]) -> Vec<Vec<u8>> {
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
    vec![
        message(67, format!("Folder changed to {folder}").as_bytes()),
        message(66, &p),
    ]
}

/// Id 100 raw block holding id 102 with the file.
fn file_body(name: &str, data: &[u8]) -> Vec<u8> {
    let mut packet = vec![gchar(102)];
    push_gint5(&mut packet, 0);
    push_gstring(&mut packet, name.as_bytes());
    packet.extend_from_slice(data);
    let n = packet.len() + 1;
    let mut body = vec![gchar(100)];
    body.extend_from_slice(&[
        gchar(((n >> 14) & 0x7F) as u8),
        gchar(((n >> 7) & 0x7F) as u8),
        gchar((n & 0x7F) as u8),
    ]);
    body.push(b'\n');
    body.extend_from_slice(&packet);
    body.push(b'\n');
    body
}

#[derive(Debug, Default)]
struct GameSeen {
    ids: Vec<u8>,
    saved_email: Option<String>,
    deleted_account: Option<String>,
    puts: Vec<(String, Vec<u8>)>,
    deleted_file: Option<String>,
    renamed: Option<(String, String)>,
}

/// One connection. Players: alice and an NPC-server player. Keeps a folder
/// `world/` with files in memory.
fn fake_gserver() -> (String, thread::JoinHandle<GameSeen>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap().to_string();
    let h = thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        s.set_read_timeout(Some(TIMEOUT)).unwrap();
        let login = zlib_decompress(&read_frame(&mut s).unwrap()).unwrap();
        let mut codec = Codec::gen5(login[1] - 0x20);
        send_msgs(
            &mut s,
            &mut codec,
            &[
                message(25, &[gchar(73)]),
                addplayer(2, "alice", "start.nw", "Alice"),
                addplayer(9, "(npcserver)", " ", "NPC-Server (Server)"),
            ],
        );
        let mut seen = GameSeen::default();
        let mut account = Account {
            account: "test1".into(),
            email: "old@example.invalid".into(),
            admin_level: 1,
            ..Default::default()
        };
        let mut files: Vec<String> = vec!["a.nw".into(), "b.txt".into()];
        while let Some(frame) = read_frame(&mut s) {
            let body = codec.decode(&frame).unwrap();
            if body.first().map(|b| b.wrapping_sub(0x20)) == Some(50) {
                // Raw block: id 93, GSTRING path, the bytes, a line feed.
                let n = read_gint3(&body[1..4]).unwrap();
                let inner = &body[5..4 + n];
                assert_eq!(inner[0].wrapping_sub(0x20), 93);
                let len = usize::from(inner[1] - 0x20);
                let path = text(&inner[2..2 + len]);
                let data = inner[2 + len..].to_vec();
                seen.ids.push(93);
                let name = path.rsplit('/').next().unwrap().to_string();
                files.push(name);
                send_msgs(
                    &mut s,
                    &mut codec,
                    &[message(67, format!("Uploaded file {path}").as_bytes())],
                );
                seen.puts.push((path, data));
                continue;
            }
            for m in split_messages(&body) {
                if m.id != 162 {
                    seen.ids.push(m.id);
                }
                let reply: Vec<Vec<u8>> = match m.id {
                    72 => {
                        let mut p = Vec::new();
                        push_gstring(&mut p, b"labrc");
                        push_gstring(&mut p, b"test1");
                        vec![message(70, &p)]
                    }
                    77 => vec![account_reply(&account)],
                    78 => {
                        let Message { payload, .. } = &m;
                        let mut i = 0;
                        let gs = |i: &mut usize| {
                            let n = usize::from(payload[*i] - 0x20);
                            let v = text(&payload[*i + 1..*i + 1 + n]);
                            *i += 1 + n;
                            v
                        };
                        gs(&mut i);
                        gs(&mut i);
                        account.email = gs(&mut i);
                        account.admin_level = payload[i + 2] - 0x20;
                        seen.saved_email = Some(account.email.clone());
                        vec![]
                    }
                    71 => {
                        seen.deleted_account = Some(text(&m.payload));
                        vec![]
                    }
                    90 => dir_reply(&text(&m.payload), &files),
                    92 => {
                        let path = text(&m.payload);
                        let name = path.rsplit('/').next().unwrap().to_string();
                        let body = file_body(&name, b"GLEVNW01\n");
                        s.write_all(&codec.encode(&body).unwrap()).unwrap();
                        vec![]
                    }
                    97 => {
                        let name = text(&m.payload);
                        files.retain(|f| *f != name);
                        seen.deleted_file = Some(name.clone());
                        vec![message(67, format!("File {name} deleted").as_bytes())]
                    }
                    98 => {
                        let mut i = 0;
                        let a = usize::from(m.payload[i] - 0x20);
                        let old = text(&m.payload[1..1 + a]);
                        i += 1 + a;
                        let b = usize::from(m.payload[i] - 0x20);
                        let new = text(&m.payload[i + 1..i + 1 + b]);
                        for f in files.iter_mut() {
                            if *f == old {
                                *f = new.clone();
                            }
                        }
                        seen.renamed = Some((old, new));
                        vec![]
                    }
                    94 => {
                        let mut p = m.payload[..2].to_vec();
                        p.extend_from_slice(b"127.0.0.1,14804");
                        vec![message(79, &p)]
                    }
                    _ => vec![],
                };
                if !reply.is_empty() {
                    send_msgs(&mut s, &mut codec, &reply);
                }
            }
        }
        seen
    });
    (addr, h)
}

fn params() -> LoginParams {
    LoginParams {
        kind: RcKind::Gen5,
        token: RcKind::Gen5.default_token().into(),
        account: "labrc".into(),
        password: "labpw".into(),
        key: 0x11,
        nickname: Some("*labrc".into()),
        identity: None,
    }
}

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
fn accounts_tab_lists_reads_saves_and_deletes() {
    let (addr, h) = fake_gserver();
    let live = LiveSession::start(addr, params(), TIMEOUT);
    let mut view = RcView::default();
    pump(&live, &mut view, |v| v.players.players.len() == 2);

    live.send(Command::AccountList("%".into()));
    pump(&live, &mut view, |v| !v.accounts.is_empty());
    assert_eq!(view.accounts, vec!["labrc", "test1"]);

    live.send(Command::AccountGet("test1".into()));
    pump(&live, &mut view, |v| v.account.is_some());
    let mut a = view.account.clone().unwrap();
    assert_eq!(a.email, "old@example.invalid");

    a.email = "new@example.invalid".into();
    a.admin_level = 3;
    view.account = None;
    live.send(Command::AccountSet(a));
    // The set is read back at once.
    pump(&live, &mut view, |v| v.account.is_some());
    let back = view.account.clone().unwrap();
    assert_eq!(back.email, "new@example.invalid");
    assert_eq!(back.admin_level, 3);

    view.accounts.clear();
    live.send(Command::AccountDelete("test1".into()));
    pump(&live, &mut view, |v| !v.accounts.is_empty());
    assert!(view.log.iter().any(|l| l == "[sent] deleted account test1"));

    drop(live);
    let seen = h.join().unwrap();
    assert_eq!(seen.saved_email.as_deref(), Some("new@example.invalid"));
    assert_eq!(seen.deleted_account.as_deref(), Some("test1"));
    // Nickname, list, get, set + get, delete + list.
    assert_eq!(seen.ids, vec![2, 72, 77, 78, 77, 71, 72]);
}

#[test]
fn files_tab_lists_gets_puts_renames_and_deletes() {
    let (addr, h) = fake_gserver();
    let live = LiveSession::start(addr, params(), TIMEOUT);
    let mut view = RcView::default();
    pump(&live, &mut view, |v| v.players.players.len() == 2);

    live.send(Command::FilesList("world/".into()));
    pump(&live, &mut view, |v| v.dir.is_some());
    assert_eq!(
        view.dir.clone().unwrap(),
        (
            "world/".to_string(),
            vec!["a.nw".to_string(), "b.txt".into()]
        )
    );

    live.send(Command::FileGet("world/a.nw".into()));
    pump(&live, &mut view, |v| v.file.is_some());
    assert_eq!(
        view.file.clone().unwrap(),
        ("world/a.nw".to_string(), b"GLEVNW01\n".to_vec())
    );

    view.dir = None;
    live.send(Command::FilePut {
        path: "world/new.txt".into(),
        data: b"hello\n".to_vec(),
    });
    pump(&live, &mut view, |v| v.dir.is_some());
    assert!(view
        .dir
        .as_ref()
        .unwrap()
        .1
        .contains(&"new.txt".to_string()));
    assert!(view
        .log
        .iter()
        .any(|l| l == "[sent] sent 6 bytes as world/new.txt"));

    view.dir = None;
    live.send(Command::FileRename {
        path: "world/b.txt".into(),
        new_name: "c.txt".into(),
    });
    pump(&live, &mut view, |v| v.dir.is_some());
    assert!(view.dir.as_ref().unwrap().1.contains(&"c.txt".to_string()));

    view.dir = None;
    live.send(Command::FileDelete("world/a.nw".into()));
    pump(&live, &mut view, |v| v.dir.is_some());
    assert!(!view.dir.as_ref().unwrap().1.contains(&"a.nw".to_string()));
    assert!(view
        .log
        .iter()
        .any(|l| l == "[sent] [server] File a.nw deleted"));

    drop(live);
    let seen = h.join().unwrap();
    assert_eq!(
        seen.puts,
        vec![("world/new.txt".into(), b"hello\n".to_vec())]
    );
    assert_eq!(seen.renamed, Some(("b.txt".into(), "c.txt".into())));
    assert_eq!(seen.deleted_file.as_deref(), Some("a.nw"));
    // A rename and a delete each list the folder first, then again after.
    assert_eq!(
        seen.ids,
        vec![2, 90, 92, 93, 90, 90, 98, 90, 90, 97, 90],
        "{:?}",
        seen.ids
    );
}

#[test]
fn find_npc_server_asks_the_server_player() {
    let (addr, h) = fake_gserver();
    let live = LiveSession::start(addr, params(), TIMEOUT);
    let mut view = RcView::default();
    pump(&live, &mut view, |v| v.players.players.len() == 2);
    live.send(Command::FindNpcServer);
    pump(&live, &mut view, |v| !v.npc_servers.is_empty());
    assert_eq!(view.npc_servers, vec!["127.0.0.1:14804"]);
    drop(live);
    assert_eq!(h.join().unwrap().ids, vec![2, 94]);
}

/// A fake NPC-server: one NPC and one class at login, then answers the
/// weapon list, a weapon, an NPC script, and records sets and deletes.
fn fake_nc() -> (String, thread::JoinHandle<Vec<Message>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap().to_string();
    let h = thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        s.set_read_timeout(Some(TIMEOUT)).unwrap();
        let login = zlib_decompress(&read_frame(&mut s).unwrap()).unwrap();
        assert!(login.starts_with(b"#NCL11012"));
        let mut codec = Codec::Plain;
        let mut row = Vec::new();
        push_gint3(&mut row, 1001);
        row.push(gchar(50));
        push_gstring(&mut row, b"door");
        row.push(gchar(52));
        push_gstring(&mut row, b"start.nw");
        send_msgs(
            &mut s,
            &mut codec,
            &[message(158, &row), message(163, b"shop")],
        );
        let mut weapons = vec!["bomb".to_string()];
        let mut seen = Vec::new();
        while let Some(frame) = read_frame(&mut s) {
            for m in split_messages(&codec.decode(&frame).unwrap()) {
                let reply = match m.id {
                    115 => {
                        let mut p = Vec::new();
                        for w in &weapons {
                            push_gstring(&mut p, w.as_bytes());
                        }
                        vec![message(167, &p)]
                    }
                    116 => {
                        let mut p = Vec::new();
                        push_gstring(&mut p, b"bomb");
                        p.push(gchar(0));
                        push_gstring(&mut p, b"wbomb1.png");
                        p.push(gchar(1));
                        push_gshort(&mut p, 7);
                        p.extend_from_slice(b"return;");
                        vec![message(33, &p)]
                    }
                    106 => {
                        let mut p = Vec::new();
                        push_gint3(&mut p, 1001);
                        p.extend_from_slice(b"// door");
                        vec![message(160, &p)]
                    }
                    117 => {
                        let n = usize::from(m.payload[0] - 0x20);
                        weapons.push(text(&m.payload[1..1 + n]));
                        vec![]
                    }
                    _ => vec![],
                };
                seen.push(m);
                if !reply.is_empty() {
                    send_msgs(&mut s, &mut codec, &reply);
                }
            }
        }
        seen
    });
    (addr, h)
}

fn pump_nc(live: &LiveNc, view: &mut NcView, done: impl Fn(&NcView) -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !done(view) {
        assert!(Instant::now() < deadline, "timed out; log: {:#?}", view.log);
        if let Some(u) = live.recv_timeout(Duration::from_millis(50)) {
            view.apply(u);
        }
    }
}

#[test]
fn nc_tab_lists_opens_and_saves() {
    let (addr, h) = fake_nc();
    let live = LiveNc::start(addr, "labrc".into(), "labpw".into(), TIMEOUT);
    let mut view = NcView::default();
    // NPCs and classes arrive by themselves; the weapon list is asked for.
    pump_nc(&live, &mut view, |v| {
        v.lists.npcs.len() == 1 && v.lists.classes.len() == 1 && v.lists.weapons.len() == 1
    });
    assert!(view.online.is_some());

    live.send(NcCommand::WeaponGet("bomb".into()));
    pump_nc(&live, &mut view, |v| v.script.is_some());
    assert_eq!(
        view.script.clone().unwrap(),
        (
            ScriptTarget::Weapon {
                name: "bomb".into(),
                image: "wbomb1.png".into()
            },
            "return;".into()
        )
    );

    live.send(NcCommand::NpcScript(1001));
    pump_nc(
        &live,
        &mut view,
        |v| matches!(&v.script, Some((ScriptTarget::Npc(1001), s)) if s == "// door"),
    );

    live.send(NcCommand::NpcScriptSet {
        id: 1001,
        script: "// door\nreturn;".into(),
    });
    live.send(NcCommand::WeaponSet {
        name: "lantern".into(),
        image: "wlantern.png".into(),
        script: "return;".into(),
    });
    pump_nc(&live, &mut view, |v| v.lists.weapons.len() == 2);
    assert_eq!(view.lists.weapons, vec!["bomb", "lantern"]);

    drop(live);
    let seen = h.join().unwrap();
    let ids: Vec<u8> = seen.iter().map(|m| m.id).collect();
    assert_eq!(ids, vec![115, 116, 106, 109, 117, 115]);
}
