//! Discovery and NPC, class and weapon commands against fake servers on
//! ephemeral loopback ports. The fakes follow `docs/PROTOCOL.md`. They are
//! not the gserver-v2 fork; that pack was not available for this run.

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, Instant};

use seihai_core::protocol::{read_gshort, split_messages, zlib_decompress, Message};
use seihai_rc::link::Codec;
use seihai_rc::nc::{
    npc_delete_message, npc_get_message, npc_script_get_message, npc_script_set_message,
    weapon_delete_message, weapon_get_message, weapon_list_message, weapon_set_message, NcSession,
    NC_CLASS_DELETE_NOTIFY, NC_CLASS_GET, NC_CLASS_NAME, NC_CLASS_SCRIPT, NC_CLASS_SET,
    NC_NPC_ATTRIBUTES, NC_NPC_REMOVED, NC_NPC_ROW, NC_NPC_SCRIPT, NC_WEAPON_NAMES,
    NC_WEAPON_SCRIPT, NPC_PROP_LEVEL, NPC_PROP_NAME, NPC_PROP_TYPE,
};
use seihai_rc::rc::{discover_npc_servers, LoginParams, RcKind};
use seihai_rc::wire::{gchar, message, push_gint3, push_gshort, push_gstring};

const TIMEOUT: Duration = Duration::from_secs(5);
const QUIET: Duration = Duration::from_millis(200);

fn read_raw_frame(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 2];
    stream.read_exact(&mut len)?;
    let mut frame = vec![0u8; usize::from(u16::from_be_bytes(len))];
    stream.read_exact(&mut frame)?;
    Ok(frame)
}

fn send_msgs(stream: &mut TcpStream, codec: &mut Codec, msgs: &[Vec<u8>]) {
    let mut body = Vec::new();
    for msg in msgs {
        body.extend_from_slice(msg);
        body.push(b'\n');
    }
    stream.write_all(&codec.encode(&body).unwrap()).unwrap();
    stream.flush().unwrap();
}

fn addplayer(id: u16, account: &str, level: &str, nick: &str) -> Vec<u8> {
    let mut payload = Vec::new();
    push_gshort(&mut payload, id);
    push_gstring(&mut payload, account.as_bytes());
    payload.push(gchar(20));
    push_gstring(&mut payload, level.as_bytes());
    payload.push(gchar(53));
    payload.push(gchar(0));
    payload.push(gchar(0));
    push_gstring(&mut payload, nick.as_bytes());
    payload.push(gchar(82));
    push_gstring(&mut payload, account.as_bytes());
    message(55, &payload)
}

fn npc_row(id: u32, name: &str, kind: &str, level: &str) -> Vec<u8> {
    let mut payload = Vec::new();
    push_gint3(&mut payload, id);
    payload.push(gchar(NPC_PROP_NAME));
    push_gstring(&mut payload, name.as_bytes());
    payload.push(gchar(NPC_PROP_TYPE));
    push_gstring(&mut payload, kind.as_bytes());
    payload.push(gchar(NPC_PROP_LEVEL));
    push_gstring(&mut payload, level.as_bytes());
    message(NC_NPC_ROW, &payload)
}

fn gint3_only(id: u32) -> Vec<u8> {
    let mut payload = Vec::new();
    push_gint3(&mut payload, id);
    payload
}

/// A one-connection fake gserver. After the welcome it answers id 94 with
/// id 79, using the address text supplied for that player id.
fn fake_game_server(
    players: Vec<Vec<u8>>,
    addresses: Vec<(u16, Vec<u8>)>,
) -> (String, thread::JoinHandle<Vec<Message>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(TIMEOUT)).unwrap();
        let login = zlib_decompress(&read_raw_frame(&mut stream).unwrap()).unwrap();
        let mut codec = match login[0].wrapping_sub(0x20) {
            6 => Codec::gen5(login[1].wrapping_sub(0x20)),
            1 => Codec::Plain,
            other => panic!("unexpected login type {other}"),
        };
        let mut welcome = vec![message(25, &[gchar(73)])];
        welcome.extend(players);
        send_msgs(&mut stream, &mut codec, &welcome);
        let mut seen = Vec::new();
        while let Ok(frame) = read_raw_frame(&mut stream) {
            let body = codec.decode(&frame).unwrap();
            let msgs = split_messages(&body);
            let queries: Vec<u16> = msgs
                .iter()
                .filter(|msg| msg.id == 94)
                .filter_map(|msg| read_gshort(&msg.payload))
                .collect();
            seen.extend(msgs);
            if queries.is_empty() {
                continue;
            }
            let replies: Vec<Vec<u8>> = queries
                .iter()
                .filter_map(|id| {
                    addresses
                        .iter()
                        .find(|(player, _)| player == id)
                        .map(|(_, text)| {
                            let mut payload = Vec::new();
                            push_gshort(&mut payload, *id);
                            payload.extend_from_slice(text);
                            message(79, &payload)
                        })
                })
                .collect();
            if !replies.is_empty() {
                send_msgs(&mut stream, &mut codec, &replies);
            }
        }
        seen
    });
    (addr, handle)
}

enum NcPlan {
    /// Send these messages as soon as the login has been read.
    Push(Vec<Vec<u8>>),
    /// Read one request and answer it.
    Reply {
        id: u8,
        payload: Vec<u8>,
        reply: Vec<Vec<u8>>,
    },
}

fn fake_nc(plan: NcPlan) -> (String, thread::JoinHandle<Vec<Message>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(TIMEOUT)).unwrap();
        let frame = read_raw_frame(&mut stream).unwrap();
        assert_eq!(frame[0], 0x78, "NC login frame is zlib, got {frame:?}");
        let login = zlib_decompress(&frame).unwrap();
        assert!(
            login.starts_with(b"#NCL11012"),
            "NC login was not plain type 3: {login:?}"
        );
        let mut codec = Codec::Plain;
        let mut seen = Vec::new();
        match plan {
            NcPlan::Push(msgs) => send_msgs(&mut stream, &mut codec, &msgs),
            NcPlan::Reply { id, payload, reply } => {
                let frame = read_raw_frame(&mut stream).expect("request frame");
                let msgs = split_messages(&codec.decode(&frame).unwrap());
                assert_eq!(msgs.len(), 1, "{msgs:?}");
                assert_eq!(msgs[0].id, id, "{msgs:?}");
                assert_eq!(msgs[0].payload, payload);
                seen = msgs;
                if !reply.is_empty() {
                    send_msgs(&mut stream, &mut codec, &reply);
                }
            }
        }
        let mut rest = Vec::new();
        let _ = stream.read_to_end(&mut rest);
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

fn server_player() -> Vec<Vec<u8>> {
    vec![
        addplayer(2, "alice", "start.nw", "Alice"),
        addplayer(4, "npc", " ", "Lab NPC (Server)"),
    ]
}

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_seihai-rc"))
        .args(args)
        .env("SEIHAI_RC_PASSWORD", "labpw")
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

#[test]
fn discover_accepts_comma_on_gen5_and_space_on_classic() {
    for (kind, text) in [
        (RcKind::Gen5, &b"127.0.0.1,14804"[..]),
        (RcKind::Classic, &b"127.0.0.1 14804"[..]),
    ] {
        let (addr, handle) = fake_game_server(server_player(), vec![(4, text.to_vec())]);
        let found = discover_npc_servers(&addr, &params(kind), TIMEOUT, QUIET).unwrap();
        assert_eq!(found.len(), 1, "{kind:?}");
        assert_eq!(found[0].player_id, 4);
        assert_eq!(found[0].nickname, "Lab NPC (Server)");
        assert_eq!(found[0].addr.raw, String::from_utf8(text.to_vec()).unwrap());
        let endpoint = found[0].addr.endpoint.clone().unwrap();
        assert_eq!(endpoint.socket_addr(), "127.0.0.1:14804");
        let seen = handle.join().unwrap();
        let queries: Vec<_> = seen.iter().filter(|msg| msg.id == 94).collect();
        assert_eq!(queries.len(), 1, "{seen:?}");
        assert_eq!(read_gshort(&queries[0].payload), Some(4));
        assert_eq!(&queries[0].payload[2..], b"location");
    }
}

#[test]
fn colon_address_does_not_open_the_nc_socket() {
    let nc = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = nc.local_addr().unwrap().port();
    let (game, handle) = fake_game_server(
        server_player(),
        vec![(4, format!("127.0.0.1:{port}").into_bytes())],
    );
    let output = cli(&[
        "nc",
        "npcs",
        "--discover",
        &game,
        "--account",
        "labrc",
        "--settle",
        "200",
        "--timeout",
        "5",
    ]);
    assert!(!output.status.success(), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("not opened"),
        "{}",
        stderr(&output)
    );
    nc.set_nonblocking(true).unwrap();
    match nc.accept() {
        Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
        other => panic!("NPC-Control socket was opened: {other:?}"),
    }
    handle.join().unwrap();
}

#[test]
fn two_addresses_do_not_open_the_nc_socket() {
    let (game, handle) = fake_game_server(
        vec![
            addplayer(4, "a", " ", "One (Server)"),
            addplayer(5, "b", " ", "Two (Server)"),
        ],
        vec![
            (4, b"127.0.0.1,14804".to_vec()),
            (5, b"127.0.0.1 14803".to_vec()),
        ],
    );
    let output = cli(&[
        "nc",
        "classes",
        "--discover",
        &game,
        "--account",
        "labrc",
        "--settle",
        "200",
        "--timeout",
        "5",
    ]);
    assert!(!output.status.success(), "{}", stderr(&output));
    let err = stderr(&output);
    assert!(err.contains("not opened"), "{err}");
    assert!(err.contains("127.0.0.1,14804"), "{err}");
    assert!(err.contains("127.0.0.1 14803"), "{err}");
    handle.join().unwrap();
}

#[test]
fn cli_discover_prints_the_raw_address_and_the_endpoint() {
    let (game, handle) = fake_game_server(server_player(), vec![(4, b"127.0.0.1,14804".to_vec())]);
    let output = cli(&[
        "nc",
        "discover",
        "--server",
        &game,
        "--account",
        "labrc",
        "--settle",
        "200",
        "--timeout",
        "5",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains("player\tnickname\taddress\tendpoint"),
        "{text}"
    );
    assert!(
        text.contains("4\tLab NPC (Server)\t127.0.0.1,14804\t127.0.0.1:14804"),
        "{text}"
    );
    handle.join().unwrap();
}

#[test]
fn discover_then_lists_npcs_from_the_space_separated_address() {
    let (nc_addr, nc_handle) = fake_nc(NcPlan::Push(vec![npc_row(
        12, "Guard", "POLICE", "start.nw",
    )]));
    let port = nc_addr.rsplit_once(':').unwrap().1;
    let (game, game_handle) = fake_game_server(
        server_player(),
        vec![(4, format!("127.0.0.1 {port}").into_bytes())],
    );
    let output = cli(&[
        "nc",
        "npcs",
        "--discover",
        &game,
        "--account",
        "labrc",
        "--settle",
        "200",
        "--timeout",
        "5",
        "--tsv",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("id\tname\ttype\tlevel\n"), "{text}");
    assert!(text.contains("12\tGuard\tPOLICE\tstart.nw\n"), "{text}");
    nc_handle.join().unwrap();
    game_handle.join().unwrap();
}

#[test]
fn npc_get_set_delete_and_script_round_trip() {
    let (addr, handle) = fake_nc(NcPlan::Reply {
        id: 103,
        payload: gint3_only(12),
        reply: vec![message(NC_NPC_ATTRIBUTES, b"hp=3")],
    });
    let output = cli(&[
        "nc",
        "npc",
        "get",
        "12",
        "--server",
        &addr,
        "--account",
        "labnc",
        "--settle",
        "200",
        "--timeout",
        "5",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "hp=3\n");
    assert_eq!(
        handle.join().unwrap()[0].payload,
        npc_get_message(12).unwrap()[1..]
    );

    let mut script_payload = gint3_only(12);
    script_payload.extend_from_slice(b"return;");
    let (addr, handle) = fake_nc(NcPlan::Reply {
        id: 106,
        payload: gint3_only(12),
        reply: vec![message(NC_NPC_SCRIPT, &script_payload)],
    });
    let output = cli(&[
        "nc",
        "npc",
        "get",
        "12",
        "--script",
        "--server",
        &addr,
        "--account",
        "labnc",
        "--settle",
        "200",
        "--timeout",
        "5",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "return;\n");
    assert_eq!(
        handle.join().unwrap()[0].payload,
        npc_script_get_message(12).unwrap()[1..]
    );

    let set = npc_script_set_message(12, "return;").unwrap();
    let (addr, handle) = fake_nc(NcPlan::Reply {
        id: 109,
        payload: set[1..].to_vec(),
        reply: vec![],
    });
    let output = cli(&[
        "nc",
        "npc",
        "set",
        "12",
        "--script",
        "return;",
        "--server",
        &addr,
        "--account",
        "labnc",
        "--settle",
        "200",
        "--timeout",
        "5",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "sent NPC script for 12\n");
    handle.join().unwrap();

    let (addr, handle) = fake_nc(NcPlan::Reply {
        id: 104,
        payload: gint3_only(12),
        reply: vec![message(NC_NPC_REMOVED, &gint3_only(12))],
    });
    let output = cli(&[
        "nc",
        "npc",
        "delete",
        "12",
        "--server",
        &addr,
        "--account",
        "labnc",
        "--settle",
        "200",
        "--timeout",
        "5",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "deleted 12\n");
    assert_eq!(
        handle.join().unwrap()[0].payload,
        npc_delete_message(12).unwrap()[1..]
    );
}

#[test]
fn class_list_get_and_set_leave_delete_unsent() {
    let (addr, handle) = fake_nc(NcPlan::Push(vec![
        message(NC_CLASS_NAME, b"Guard"),
        message(NC_CLASS_DELETE_NOTIFY, b"Old"),
    ]));
    let output = cli(&[
        "nc",
        "classes",
        "--server",
        &addr,
        "--account",
        "labnc",
        "--settle",
        "200",
        "--timeout",
        "5",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "Guard\n");
    handle.join().unwrap();

    let (addr, handle) = fake_nc(NcPlan::Reply {
        id: NC_CLASS_GET,
        payload: b"Guard".to_vec(),
        reply: vec![message(NC_CLASS_SCRIPT, b"%Guardreturn;")],
    });
    let output = cli(&[
        "nc",
        "class",
        "get",
        "Guard",
        "--server",
        &addr,
        "--account",
        "labnc",
        "--settle",
        "200",
        "--timeout",
        "5",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "return;\n");
    handle.join().unwrap();

    let (addr, handle) = fake_nc(NcPlan::Reply {
        id: NC_CLASS_SET,
        payload: b"%Guardreturn;".to_vec(),
        reply: vec![],
    });
    let output = cli(&[
        "nc",
        "class",
        "set",
        "Guard",
        "--script",
        "return;",
        "--server",
        &addr,
        "--account",
        "labnc",
        "--settle",
        "200",
        "--timeout",
        "5",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "sent class Guard\n");
    handle.join().unwrap();

    let output = cli(&["nc", "class", "delete", "Guard"]);
    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(err.contains("still needs a lab check"), "{err}");
    assert!(err.contains("119"), "{err}");
}

#[test]
fn weapon_list_get_set_and_delete() {
    let list = weapon_list_message();
    let (addr, handle) = fake_nc(NcPlan::Reply {
        id: list[0].wrapping_sub(0x20),
        payload: vec![],
        reply: vec![message(NC_WEAPON_NAMES, b"%sword#bow")],
    });
    let output = cli(&[
        "nc",
        "weapons",
        "--server",
        &addr,
        "--account",
        "labnc",
        "--settle",
        "200",
        "--timeout",
        "5",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "sword\nbow\n");
    handle.join().unwrap();

    let get = weapon_get_message("sword").unwrap();
    let mut script = Vec::new();
    push_gstring(&mut script, b"sword");
    push_gstring(&mut script, b"sword.png");
    push_gshort(&mut script, 7);
    script.extend_from_slice(b"return;");
    let (addr, handle) = fake_nc(NcPlan::Reply {
        id: get[0].wrapping_sub(0x20),
        payload: get[1..].to_vec(),
        reply: vec![message(NC_WEAPON_SCRIPT, &script)],
    });
    let output = cli(&[
        "nc",
        "weapon",
        "get",
        "sword",
        "--server",
        &addr,
        "--account",
        "labnc",
        "--settle",
        "200",
        "--timeout",
        "5",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "return;\n");
    assert!(
        stderr(&output).contains("image: sword.png"),
        "{}",
        stderr(&output)
    );
    handle.join().unwrap();

    let set = weapon_set_message("sword", "sword.png", "return;").unwrap();
    let (addr, handle) = fake_nc(NcPlan::Reply {
        id: set[0].wrapping_sub(0x20),
        payload: set[1..].to_vec(),
        reply: vec![],
    });
    let output = cli(&[
        "nc",
        "weapon",
        "set",
        "sword",
        "--image",
        "sword.png",
        "--script",
        "return;",
        "--server",
        &addr,
        "--account",
        "labnc",
        "--settle",
        "200",
        "--timeout",
        "5",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "sent weapon sword\n");
    handle.join().unwrap();

    let delete = weapon_delete_message("sword").unwrap();
    let (addr, handle) = fake_nc(NcPlan::Reply {
        id: delete[0].wrapping_sub(0x20),
        payload: delete[1..].to_vec(),
        reply: vec![],
    });
    let output = cli(&[
        "nc",
        "weapon",
        "delete",
        "sword",
        "--server",
        &addr,
        "--account",
        "labnc",
        "--settle",
        "200",
        "--timeout",
        "5",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "sent weapon delete for sword\n");
    handle.join().unwrap();
}

#[test]
fn a_script_line_break_is_refused_before_connect() {
    let nc = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = nc.local_addr().unwrap().to_string();
    nc.set_nonblocking(true).unwrap();
    let path = std::env::temp_dir().join("seihai-rc-script-break.txt");
    std::fs::write(&path, "line\nbreak").unwrap();
    let output = cli(&[
        "nc",
        "npc",
        "set",
        "12",
        "--script-file",
        path.to_str().unwrap(),
        "--server",
        &addr,
        "--account",
        "labnc",
    ]);
    assert!(!output.status.success(), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("still needs a lab check"),
        "{}",
        stderr(&output)
    );
    match nc.accept() {
        Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
        other => panic!("connected despite the line break: {other:?}"),
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn direct_nc_login_still_reports_a_level_list_id() {
    let (addr, handle) = fake_nc(NcPlan::Push(vec![message(80, b"start.nw")]));
    let mut session = NcSession::login(&addr, "labnc", "labpw", TIMEOUT).unwrap();
    let event = session
        .next_event(Instant::now() + TIMEOUT)
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        seihai_rc::nc::NcEvent::Message { id: 80, .. }
    ));
    session.close();
    handle.join().unwrap();
}
