//! `seihai-rc`: the command line.

use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use seihai_rc::listserver::{decode_server_key, fetch_rc_list, fetch_rc_list_secure, pick};
use seihai_rc::nc::{
    class_delete_message, class_get_message, class_set_message, npc_delete_message,
    npc_get_message, npc_script_get_message, npc_script_set_message, weapon_delete_message,
    weapon_get_message, weapon_list_message, weapon_set_message, NcEvent, NcSession,
};
use seihai_rc::players::{
    admin_message, ban_get, ban_set, comments_get, comments_set, disconnect_player, parse_ban,
    parse_comments, parse_rights, plo as players_plo, private_admin_message, private_message,
    resolve, rights_get, rights_set, warp_player, Rights,
};
use seihai_rc::rc::{
    discover_npc_servers, Event, LoginParams, NpcServerFound, PlayerEntry, RcKind, Session,
};

const USAGE: &str = "\
seihai-rc: Graal Remote Control client (stage 1, player actions, NPC Control)

USAGE:
  seihai-rc servers --listserver HOST:PORT --account NAME [options]
  seihai-rc login   (--server HOST:PORT | --listserver HOST:PORT --pick NAME|N)
                    --account NAME [--for SECS] [--say TEXT] [options]
  seihai-rc players list (--server ... | --listserver ... --pick ...)
                    --account NAME [--tsv] [--settle MS] [options]
  seihai-rc players pm WHO[,WHO...] TEXT... (--server ... | --listserver ... --pick ...)
  seihai-rc players admin-pm WHO TEXT... (--server ... | --listserver ... --pick ...)
  seihai-rc players say TEXT... (--server ... | --listserver ... --pick ...)
  seihai-rc players comments ACCOUNT [--set TEXT | --set-file PATH] (--server ... | ...)
  seihai-rc players ban ACCOUNT [--set on|off [--reason TEXT]] (--server ... | ...)
  seihai-rc players rights ACCOUNT [--set N] [--ip-range R] [--set-file PATH] (--server ...)
  seihai-rc players kick WHO (--server ... | --listserver ... --pick ...)
  seihai-rc players warp WHO LEVEL X Y (--server ... | --listserver ... --pick ...)
  seihai-rc nc discover --server HOST:PORT --account NAME [--settle MS] [options]
  seihai-rc nc login --server HOST:PORT --account NAME [--for SECS] [options]
  seihai-rc nc npcs (--server HOST:PORT | --discover HOST:PORT) --account NAME
  seihai-rc nc npc get ID [--script] (--server ... | --discover ...) --account NAME
  seihai-rc nc npc set ID --script TEXT (--server ... | --discover ...) --account NAME
  seihai-rc nc npc delete ID (--server ... | --discover ...) --account NAME
  seihai-rc nc classes (--server ... | --discover ...) --account NAME
  seihai-rc nc class get NAME (--server ... | --discover ...) --account NAME
  seihai-rc nc class set NAME --script TEXT (--server ... | --discover ...) --account NAME
  seihai-rc nc class delete NAME (--server ... | --discover ...) --account NAME
  seihai-rc nc weapons (--server ... | --discover ...) --account NAME
  seihai-rc nc weapon get NAME (--server ... | --discover ...) --account NAME
  seihai-rc nc weapon set NAME --image IMG --script TEXT (--server ... | --discover ...)
  seihai-rc nc weapon delete NAME (--server ... | --discover ...) --account NAME

  servers       Log in to the listserver's RC list and print every server.
  login         Log in to a gserver and print server messages and the RC
                chat for --for seconds (default 5; 0 = until the server
                closes the link). --say sends one RC chat line first.
  players list  Log in and print the players and RCs online.
  players pm    Private message (id 28) to one or more players or RCs.
  players say   Admin message to everyone online (id 63).
  players admin-pm
                Admin message to one player or RC (id 64).
  players comments
                Print an account's staff comments (id 85, reply 63), one
                line each. --set replaces them (id 86); --set-file reads
                them from a file, one comment per line. Read back after.
  players ban   Print whether an account is banned on this server, and
                why (id 87, reply 64). --set on|off with --reason changes
                it (id 88) and reads it back. The gserver-v2 fork hung
                after a ban set on an online player in the 2026-11-04 lab.
  players rights
                Print an account's rights number and set bits, IP range
                and folder rights (id 83, reply 62). --set N, --ip-range
                and --set-file (folder rights, one per line) change those
                fields, keep the rest, send id 84 and read them back.
                Bit 0 is warp-to-xy and bit 9 admin message.
  players kick  Disconnect a player (id 61) and wait for it to leave (56).
  players warp  Warp a player (id 82) to LEVEL at X Y, in tiles; halves
                such as 30.5 are allowed.
                WHO is a player id or an account name; the client reads the
                player list first (--settle) and refuses a WHO not in it.
                The payloads for 28, 61, 63, 64 and 82 still need a fork lab
                check; see docs/PROTOCOL.md.
  nc discover   Log in to the game server, ask where each \"(Server)\" player
                is listening (ids 94 and 79), and print the address text.
                This does not open the NPC-Control socket.
  nc login      Log in to an NPC-server (type 3, plain zlib) and print
                message ids for --for seconds.
  nc npcs       List NPC rows the NPC-server sends (id 158).
  nc npc        get reads attributes (id 103, reply 157). get --script reads
                the script (id 106, reply 160). set writes the script (id 109).
                delete removes the NPC (id 104) and waits for id 159.
  nc classes    List class names the NPC-server sends (id 163).
  nc class      get opens a class (id 112, reply 162). set saves it (id 113).
                delete sends id 119 (GSTRING name, as class set does) and
                waits for id 188. A bare class name is not sent.
  nc weapons    Ask for the weapon list (id 115, reply 167).
  nc weapon     get opens a weapon (id 116, reply 33). set saves it (id 117).
                delete sends id 118.

  For every nc command except discover, --server is the NPC-server.
  --discover is the game server: the client logs in there, reads one id 79
  address, and only then opens the NPC-Control socket. Give one of those
  two, not both.

OPTIONS:
  --password-env VAR   Read the password from this environment variable
                       (default SEIHAI_RC_PASSWORD). Passwords are never
                       taken on the command line.
  --rc gen5|classic    Login type: gen5 (type 6, default) or classic
                       (type 1, the 2000 to 2002 RCs, plain zlib).
  --token TOKEN        8-byte version token (default GSERV025 for gen5,
                       GSERV023 for classic).
  --nick NAME          Nickname to send after the login.
  --timeout SECS       Connect and login timeout (default 10).
  --settle MS          players: wait until no message came for this
                       long before printing or acting (default 1000).
  --discover HOST:PORT On nc commands other than discover: game server to
                       ask for the NPC-server address before connecting.
  --script TEXT        Script for npc set, class set and weapon set.
                       npc get --script reads the script instead of the
                       attributes. A script with a line break is not sent.
  --script-file PATH   Read the script from a file, instead of --script.
  --image NAME         Weapon image for weapon set.
  --set VALUE          players comments: the new comments (one line).
                       players ban: on or off.
                       players rights: the rights number.
  --set-file PATH      players comments: the new comments, one per line.
                       players rights: the folder rights, one per line.
  --ip-range RANGE     players rights: the new IP range.
  --reason TEXT        players ban --set: the reason (default empty).
  --secure-login       Ask the listserver for the RC list through the
                       secure handshake (id 5, token SEI00001). The account
                       and password go sealed. Needs --server-key.
  --server-key KEY     The listserver's Ed25519 public key, unpadded
                       base64. The password is not sent if it does not match.
  --tsv                players list and nc npcs: tab-separated, with a header.
  -v, --verbose        Also show the ids of messages not printed as text.
  -V, --version        Print the version.
";

#[derive(Default)]
struct Opts {
    cmd: Vec<String>,
    server: Option<String>,
    listserver: Option<String>,
    pick: Option<String>,
    account: Option<String>,
    password_env: Option<String>,
    rc: Option<String>,
    token: Option<String>,
    nick: Option<String>,
    timeout: Option<u64>,
    settle: Option<u64>,
    for_secs: Option<u64>,
    say: Option<String>,
    discover: Option<String>,
    script: Option<String>,
    script_file: Option<String>,
    image: Option<String>,
    set: Option<String>,
    set_file: Option<String>,
    reason: Option<String>,
    ip_range: Option<String>,
    server_key: Option<String>,
    script_flag: bool,
    secure_login: bool,
    tsv: bool,
    verbose: bool,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Opts, String> {
    let mut o = Opts::default();
    let mut it = args.peekable();
    while let Some(a) = it.next() {
        macro_rules! val {
            ($name:expr) => {
                it.next()
                    .ok_or_else(|| format!("{} needs a value", $name))?
            };
        }
        let num = |name: &str, v: String| {
            v.parse::<u64>()
                .map_err(|_| format!("{name}: not a number: {v}"))
        };
        match a.as_str() {
            "--server" => o.server = Some(val!(&a)),
            "--listserver" => o.listserver = Some(val!(&a)),
            "--pick" => o.pick = Some(val!(&a)),
            "--account" => o.account = Some(val!(&a)),
            "--password-env" => o.password_env = Some(val!(&a)),
            "--rc" => o.rc = Some(val!(&a)),
            "--token" => o.token = Some(val!(&a)),
            "--nick" => o.nick = Some(val!(&a)),
            "--say" => o.say = Some(val!(&a)),
            "--discover" => o.discover = Some(val!(&a)),
            "--script" => {
                let takes_value = it.peek().is_some_and(|next| !next.starts_with('-'));
                if takes_value {
                    o.script = Some(val!("--script"));
                } else {
                    o.script_flag = true;
                }
            }
            "--script-file" => o.script_file = Some(val!(&a)),
            "--image" => o.image = Some(val!(&a)),
            "--set" => o.set = Some(val!(&a)),
            "--set-file" => o.set_file = Some(val!(&a)),
            "--reason" => o.reason = Some(val!(&a)),
            "--ip-range" => o.ip_range = Some(val!(&a)),
            "--server-key" => o.server_key = Some(val!(&a)),
            "--secure-login" => o.secure_login = true,
            "--timeout" => o.timeout = Some(num(&a, val!(&a))?),
            "--settle" => o.settle = Some(num(&a, val!(&a))?),
            "--for" => o.for_secs = Some(num(&a, val!(&a))?),
            "--tsv" => o.tsv = true,
            "-v" | "--verbose" => o.verbose = true,
            s if s.starts_with('-') => return Err(format!("unknown option {s}")),
            _ => o.cmd.push(a),
        }
    }
    Ok(o)
}

struct Common {
    kind: RcKind,
    token: String,
    account: String,
    password: String,
    timeout: Duration,
    key: u8,
}

fn common(o: &Opts) -> Result<Common, String> {
    let kind = match &o.rc {
        None => RcKind::Gen5,
        Some(s) => {
            RcKind::parse(s).ok_or_else(|| format!("--rc: expected gen5 or classic, got {s}"))?
        }
    };
    let token = o
        .token
        .clone()
        .unwrap_or_else(|| kind.default_token().to_string());
    if token.len() != 8 {
        return Err(format!("--token must be 8 bytes, got {token:?}"));
    }
    let account = o.account.clone().ok_or("--account is required")?;
    let var = o
        .password_env
        .clone()
        .unwrap_or_else(|| "SEIHAI_RC_PASSWORD".into());
    let password = std::env::var(&var)
        .map_err(|_| format!("set the password in the environment variable {var}"))?;
    // Any key from 0 to 127 works; vary it per run like the clients do.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let key = (nanos >> 10) as u8 & 0x7F;
    Ok(Common {
        kind,
        token,
        account,
        password,
        timeout: Duration::from_secs(o.timeout.unwrap_or(10)),
        key,
    })
}

/// The gserver to log in to: `--server`, or `--pick` from the RC list.
fn target(o: &Opts, c: &Common) -> Result<String, String> {
    match (&o.server, &o.listserver, &o.pick) {
        (Some(s), None, None) => Ok(s.clone()),
        (None, Some(ls), Some(which)) => {
            let reply = fetch_list(o, c, ls)?;
            let s = pick(&reply.servers, which).ok_or_else(|| {
                format!("no single server matches {which:?} in the list from {ls}")
            })?;
            eprintln!("picked {} at {}:{}", s.name, s.host, s.port);
            Ok(format!("{}:{}", s.host, s.port))
        }
        _ => Err("give either --server HOST:PORT, or --listserver HOST:PORT and --pick".into()),
    }
}

fn login(o: &Opts, c: &Common) -> Result<(Session, String), String> {
    let addr = target(o, c)?;
    let p = LoginParams {
        kind: c.kind,
        token: c.token.clone(),
        account: c.account.clone(),
        password: c.password.clone(),
        key: c.key,
        nickname: o.nick.clone(),
    };
    let s =
        Session::login(&addr, &p, c.timeout).map_err(|e| format!("{addr}: login failed: {e}"))?;
    eprintln!("logged in to {addr} as {}", c.account);
    Ok((s, addr))
}

/// One `login` line for an arrival or a props refresh.
fn player_line(tag: &str, e: &PlayerEntry) -> String {
    format!(
        "[{tag}] {} {} ({})",
        e.id,
        e.account,
        if e.is_rc() { "RC" } else { e.level.as_str() }
    )
}

fn print_event(ev: &Event, verbose: bool) {
    match ev {
        Event::Signature => {}
        Event::Disconnect(why) => println!("[disconnected] {why}"),
        Event::RcChat(line) => println!("[rc] {line}"),
        Event::AdminMessage(m) => println!("[admin] {}", m.replace('\u{a7}', ": ")),
        Event::PrivateMessage { from, text } => println!("[pm from {from}] {text}"),
        Event::AddPlayer(e) => println!("{}", player_line("online", e)),
        Event::UpdatePlayer(e) => println!("{}", player_line("update", e)),
        Event::DelPlayer(id) => println!("[offline] {id}"),
        Event::NpcServerAddr(addr) => {
            println!("[npc-server] {} {}", addr.player_id, addr.raw);
        }
        Event::Other(id, p) if verbose => eprintln!("[id {id}] {} bytes", p.len()),
        Event::Other(..) => {}
    }
}

fn fetch_list(o: &Opts, c: &Common, ls: &str) -> Result<seihai_core::protocol::ListReply, String> {
    let reply = if o.secure_login {
        let text = o
            .server_key
            .as_deref()
            .ok_or("--secure-login needs --server-key KEY")?;
        let key =
            decode_server_key(text).ok_or("--server-key is not an unpadded base64 Ed25519 key")?;
        fetch_rc_list_secure(
            ls,
            &c.token,
            &c.account,
            &c.password,
            c.key,
            &key,
            c.timeout,
        )
    } else if o.server_key.is_some() {
        return Err("--server-key is only used with --secure-login".into());
    } else {
        fetch_rc_list(ls, &c.token, &c.account, &c.password, c.key, c.timeout)
    };
    reply.map_err(|e| format!("{ls}: {e}"))
}

fn cmd_servers(o: &Opts) -> Result<(), String> {
    let c = common(o)?;
    let ls = o.listserver.as_deref().ok_or("--listserver is required")?;
    let reply = fetch_list(o, &c, ls)?;
    if let Some(st) = &reply.status {
        for line in st.lines() {
            eprintln!("{line}");
        }
    }
    for (i, s) in reply.servers.iter().enumerate() {
        println!("{}", s.display_line(i + 1).trim_start());
    }
    Ok(())
}

fn cmd_login(o: &Opts) -> Result<(), String> {
    let c = common(o)?;
    let (mut s, _) = login(o, &c)?;
    if let Some(line) = &o.say {
        s.send_rc_chat(line).map_err(|e| e.to_string())?;
    }
    let secs = o.for_secs.unwrap_or(5);
    let deadline = if secs == 0 {
        Instant::now() + Duration::from_secs(u64::from(u32::MAX))
    } else {
        Instant::now() + Duration::from_secs(secs)
    };
    loop {
        match s.next_event(deadline).map_err(|e| e.to_string())? {
            Some(ev) => {
                print_event(&ev, o.verbose);
                if matches!(ev, Event::Disconnect(_)) {
                    break;
                }
            }
            None => {
                if s.is_closed() {
                    println!("[closed] the server closed the link");
                }
                break;
            }
        }
    }
    s.close();
    Ok(())
}

fn row(e: &PlayerEntry, tsv: bool) -> String {
    let kind = if e.is_rc() { "rc" } else { "player" };
    if tsv {
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            e.id, kind, e.account, e.nickname, e.level, e.status, e.community
        )
    } else {
        format!(
            "{:>5}  {:<6}  {:<20}  {:<20}  {}",
            e.id, kind, e.account, e.nickname, e.level
        )
    }
}

fn cmd_nc_login(o: &Opts) -> Result<(), String> {
    let c = common(o)?;
    if o.listserver.is_some() || o.pick.is_some() {
        return Err(
            "nc login takes --server HOST:PORT; the RC list does not carry the NPC-server address"
                .into(),
        );
    }
    let addr = o
        .server
        .clone()
        .ok_or("nc login needs --server HOST:PORT")?;
    let mut s = NcSession::login(&addr, &c.account, &c.password, c.timeout)
        .map_err(|e| format!("{addr}: NC login failed: {e}"))?;
    eprintln!("connected to {addr} as {} (NPC Control)", c.account);
    let secs = o.for_secs.unwrap_or(5);
    let deadline = if secs == 0 {
        Instant::now() + Duration::from_secs(u64::from(u32::MAX))
    } else {
        Instant::now() + Duration::from_secs(secs)
    };
    loop {
        match s.next_event(deadline).map_err(|e| e.to_string())? {
            Some(NcEvent::Disconnect(why)) => {
                println!("[disconnected] {why}");
                s.close();
                return Err(format!("NPC-server disconnected: {why}"));
            }
            Some(NcEvent::Message { id, payload }) => {
                println!("[nc] id {id} ({} bytes)", payload.len());
            }
            Some(other) => println!("{}", nc_verbose(&other)),
            None => {
                if s.is_closed() {
                    println!("[closed] the server closed the link");
                }
                break;
            }
        }
    }
    s.close();
    Ok(())
}

fn cmd_players_list(o: &Opts) -> Result<(), String> {
    let c = common(o)?;
    let (mut s, _) = login(o, &c)?;
    let quiet = Duration::from_millis(o.settle.unwrap_or(1000));
    s.settle(quiet, Instant::now() + c.timeout)
        .map_err(|e| e.to_string())?;
    if o.tsv {
        println!("id\tkind\taccount\tnickname\tlevel\tstatus\tcommunity");
    } else {
        println!(
            "{:>5}  {:<6}  {:<20}  {:<20}  level",
            "id", "kind", "account", "nickname"
        );
    }
    for e in s.players.players.values() {
        println!("{}", row(e, o.tsv));
    }
    if !o.tsv {
        let n = s.players.players.values().filter(|e| !e.is_rc()).count();
        let r = s.players.players.len() - n;
        println!("{n} player(s), {r} other RC(s)");
    }
    s.close();
    Ok(())
}

/// Log in and let the player list arrive, for a `players` action.
fn players_session(o: &Opts) -> Result<(Session, Common), String> {
    let c = common(o)?;
    let (mut s, _) = login(o, &c)?;
    let quiet = Duration::from_millis(o.settle.unwrap_or(1000));
    s.settle(quiet, Instant::now() + c.timeout)
        .map_err(|e| e.to_string())?;
    Ok((s, c))
}

/// Send one action, then read what the server says for `--settle` ms.
fn send_and_settle(
    s: &mut Session,
    o: &Opts,
    c: &Common,
    msg: Vec<u8>,
) -> Result<Vec<Event>, String> {
    s.send_messages(&[msg]).map_err(|e| e.to_string())?;
    let quiet = Duration::from_millis(o.settle.unwrap_or(1000));
    let events = s
        .settle(quiet, Instant::now() + c.timeout)
        .map_err(|e| e.to_string())?;
    for ev in &events {
        if let Event::Disconnect(why) = ev {
            return Err(format!("the server closed the link: {why}"));
        }
        if o.verbose {
            print_event(ev, true);
        }
    }
    Ok(events)
}

fn cmd_players_pm(o: &Opts, who: &str, words: &[&str]) -> Result<(), String> {
    let text = words.join(" ");
    let (mut s, c) = players_session(o)?;
    let mut ids = Vec::new();
    for one in who.split(',') {
        let e = resolve(&s.players, one.trim())?;
        if !ids.contains(&e.id) {
            ids.push(e.id);
        }
    }
    let msg = private_message(&ids, &text).map_err(|e| e.to_string())?;
    send_and_settle(&mut s, o, &c, msg)?;
    let list: Vec<String> = ids.iter().map(u16::to_string).collect();
    println!("sent private message to {}", list.join(","));
    s.close();
    Ok(())
}

fn cmd_players_say(o: &Opts, words: &[&str]) -> Result<(), String> {
    let msg = admin_message(&words.join(" ")).map_err(|e| e.to_string())?;
    let (mut s, c) = players_session(o)?;
    send_and_settle(&mut s, o, &c, msg)?;
    println!("sent admin message");
    s.close();
    Ok(())
}

fn cmd_players_admin_pm(o: &Opts, who: &str, words: &[&str]) -> Result<(), String> {
    let text = words.join(" ");
    let (mut s, c) = players_session(o)?;
    let e = resolve(&s.players, who)?.clone();
    let msg = private_admin_message(e.id, &text).map_err(|e| e.to_string())?;
    send_and_settle(&mut s, o, &c, msg)?;
    println!("sent admin message to {} {}", e.id, e.account);
    s.close();
    Ok(())
}

/// Read events until a message `id` arrives that `parse` reads for
/// `account`, or the timeout passes.
fn await_reply<T>(
    s: &mut Session,
    c: &Common,
    id: u8,
    account: &str,
    parse: impl Fn(&[u8]) -> Option<(String, T)>,
) -> Result<T, String> {
    let deadline = Instant::now() + c.timeout;
    loop {
        match s.next_event(deadline).map_err(|e| e.to_string())? {
            Some(Event::Other(got, payload)) if got == id => {
                if let Some((who, value)) = parse(&payload) {
                    if who.eq_ignore_ascii_case(account) {
                        return Ok(value);
                    }
                }
            }
            Some(Event::Disconnect(why)) => {
                return Err(format!("the server closed the link: {why}"))
            }
            Some(_) => {}
            None => {
                return Err(format!(
                    "no reply (id {id}) for {account} before the timeout"
                ))
            }
        }
    }
}

fn read_comments(s: &mut Session, c: &Common, account: &str) -> Result<Vec<String>, String> {
    let msg = comments_get(account).map_err(|e| e.to_string())?;
    s.send_messages(&[msg]).map_err(|e| e.to_string())?;
    await_reply(s, c, players_plo::RC_PLAYERCOMMENTSGET, account, |p| {
        parse_comments(p).map(|r| (r.account, r.lines))
    })
}

fn cmd_players_comments(o: &Opts, account: &str) -> Result<(), String> {
    let new = match (&o.set, &o.set_file) {
        (Some(_), Some(_)) => return Err("give either --set or --set-file".into()),
        (Some(text), None) => Some(text.clone()),
        (None, Some(path)) => {
            Some(std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?)
        }
        (None, None) => None,
    };
    let set = match &new {
        Some(text) => Some(
            comments_set(account, text.trim_end_matches(['\n', '\r']))
                .map_err(|e| e.to_string())?,
        ),
        None => None,
    };
    let c = common(o)?;
    let (mut s, _) = login(o, &c)?;
    if let Some(msg) = set {
        s.send_messages(&[msg]).map_err(|e| e.to_string())?;
    }
    let lines = read_comments(&mut s, &c, account)?;
    s.close();
    for line in &lines {
        println!("{line}");
    }
    if let Some(text) = new {
        let want: Vec<&str> = text.trim_end_matches(['\n', '\r']).lines().collect();
        if lines != want {
            return Err(format!(
                "sent the comments for {account}, but the server read back {} line(s), not {}",
                lines.len(),
                want.len()
            ));
        }
        eprintln!("set the comments for {account}");
    }
    Ok(())
}

fn cmd_players_ban(o: &Opts, account: &str) -> Result<(), String> {
    let want = match o.set.as_deref() {
        None => {
            if o.reason.is_some() {
                return Err("--reason needs --set on or --set off".into());
            }
            None
        }
        Some("on") => Some(true),
        Some("off") => Some(false),
        Some(other) => return Err(format!("--set: expected on or off, got {other}")),
    };
    let reason = o.reason.clone().unwrap_or_default();
    let set = match want {
        Some(on) => Some(ban_set(account, on, &reason).map_err(|e| e.to_string())?),
        None => None,
    };
    let get = ban_get(account).map_err(|e| e.to_string())?;
    let c = common(o)?;
    let (mut s, _) = login(o, &c)?;
    if let Some(msg) = set {
        s.send_messages(&[msg]).map_err(|e| e.to_string())?;
    }
    s.send_messages(&[get]).map_err(|e| e.to_string())?;
    let ban = await_reply(&mut s, &c, players_plo::RC_PLAYERBANGET, account, |p| {
        parse_ban(p).map(|b| (b.account.clone(), b))
    })?;
    s.close();
    println!(
        "{}: {}",
        ban.account,
        if ban.banned { "banned" } else { "not banned" }
    );
    for line in &ban.reason {
        println!("reason: {line}");
    }
    if let Some(on) = want {
        if ban.banned != on {
            return Err(format!(
                "sent the ban for {account}, but the server reads it back as {}",
                if ban.banned { "on" } else { "off" }
            ));
        }
    }
    Ok(())
}

fn read_rights(s: &mut Session, c: &Common, account: &str) -> Result<Rights, String> {
    let msg = rights_get(account).map_err(|e| e.to_string())?;
    s.send_messages(&[msg]).map_err(|e| e.to_string())?;
    await_reply(s, c, players_plo::RC_PLAYERRIGHTSGET, account, |p| {
        parse_rights(p).map(|r| (r.account.clone(), r))
    })
}

/// The bits set in `rights`, as `0, 9`.
fn bit_list(rights: u32) -> String {
    let bits: Vec<String> = (0..32)
        .filter(|b| rights & (1 << b) != 0)
        .map(|b| b.to_string())
        .collect();
    if bits.is_empty() {
        "none".into()
    } else {
        bits.join(", ")
    }
}

fn cmd_players_rights(o: &Opts, account: &str) -> Result<(), String> {
    let new_rights = match &o.set {
        Some(v) => Some(
            v.parse::<u32>()
                .map_err(|_| format!("--set: not a rights number: {v}"))?,
        ),
        None => None,
    };
    let new_folders = match &o.set_file {
        Some(path) => Some(
            std::fs::read_to_string(path)
                .map_err(|e| format!("{path}: {e}"))?
                .lines()
                .map(str::to_string)
                .collect::<Vec<_>>(),
        ),
        None => None,
    };
    let change = new_rights.is_some() || new_folders.is_some() || o.ip_range.is_some();
    let c = common(o)?;
    let (mut s, _) = login(o, &c)?;
    let mut r = read_rights(&mut s, &c, account)?;
    if change {
        let mut want = r.clone();
        if let Some(n) = new_rights {
            want.rights = n;
        }
        if let Some(ip) = &o.ip_range {
            want.ip_range = ip.clone();
        }
        if let Some(lines) = new_folders {
            want.folders = lines;
        }
        let msg = rights_set(&want).map_err(|e| e.to_string())?;
        s.send_messages(&[msg]).map_err(|e| e.to_string())?;
        r = read_rights(&mut s, &c, account)?;
        if r.rights != want.rights || r.ip_range != want.ip_range || r.folders != want.folders {
            s.close();
            return Err(format!(
                "sent the rights for {account}, but the server read back rights {}, \
                 IP range {:?} and {} folder line(s)",
                r.rights,
                r.ip_range,
                r.folders.len()
            ));
        }
    }
    s.close();
    println!("account: {}", r.account);
    println!("rights: {} (bits: {})", r.rights, bit_list(r.rights));
    println!("ip range: {}", r.ip_range);
    for line in &r.folders {
        println!("folder: {line}");
    }
    Ok(())
}

fn cmd_players_kick(o: &Opts, who: &str) -> Result<(), String> {
    let (mut s, c) = players_session(o)?;
    let e = resolve(&s.players, who)?.clone();
    let msg = disconnect_player(e.id).map_err(|e| e.to_string())?;
    let events = send_and_settle(&mut s, o, &c, msg)?;
    s.close();
    if events
        .iter()
        .any(|ev| matches!(ev, Event::DelPlayer(id) if *id == e.id))
    {
        println!("disconnected {} {}", e.id, e.account);
        Ok(())
    } else {
        Err(format!(
            "sent disconnect for {} {}, but no DELPLAYER (id 56) for it arrived",
            e.id, e.account
        ))
    }
}

fn parse_tiles(axis: &str, v: &str) -> Result<f32, String> {
    v.parse::<f32>()
        .map_err(|_| format!("{axis}: not a number: {v}"))
}

fn cmd_players_warp(o: &Opts, who: &str, level: &str, x: &str, y: &str) -> Result<(), String> {
    let (x, y) = (parse_tiles("X", x)?, parse_tiles("Y", y)?);
    // Check the bytes before logging in, so a bad X or Y costs nothing.
    warp_player(0, level, x, y).map_err(|e| e.to_string())?;
    let (mut s, c) = players_session(o)?;
    let e = resolve(&s.players, who)?.clone();
    if e.is_rc() {
        return Err(format!("{} {} is an RC, not a player", e.id, e.account));
    }
    let msg = warp_player(e.id, level, x, y).map_err(|e| e.to_string())?;
    let events = send_and_settle(&mut s, o, &c, msg)?;
    s.close();
    println!("sent warp for {} {} to {level} {x} {y}", e.id, e.account);
    for ev in &events {
        if let Event::UpdatePlayer(now) = ev {
            if now.id == e.id {
                println!("{}", player_line("update", now));
            }
        }
    }
    Ok(())
}

fn login_params(o: &Opts, c: &Common) -> LoginParams {
    LoginParams {
        kind: c.kind,
        token: c.token.clone(),
        account: c.account.clone(),
        password: c.password.clone(),
        key: c.key,
        nickname: o.nick.clone(),
    }
}

fn discover_at(addr: &str, o: &Opts, c: &Common) -> Result<Vec<NpcServerFound>, String> {
    let quiet = Duration::from_millis(o.settle.unwrap_or(1000));
    discover_npc_servers(addr, &login_params(o, c), c.timeout, quiet)
        .map_err(|e| format!("{addr}: {e}"))
}

fn nc_address(o: &Opts, c: &Common) -> Result<String, String> {
    match (&o.server, &o.discover) {
        (Some(_), Some(_)) => Err(
            "give either --server HOST:PORT for the NPC-server, or --discover HOST:PORT for the game server"
                .into(),
        ),
        (Some(server), None) => Ok(server.clone()),
        (None, Some(game)) => one_npc_endpoint(&discover_at(game, o, c)?),
        (None, None) => Err(
            "give --server HOST:PORT (the NPC-server) or --discover HOST:PORT (the game server)"
                .into(),
        ),
    }
}

fn one_npc_endpoint(found: &[NpcServerFound]) -> Result<String, String> {
    if let [item] = found {
        if let Some(endpoint) = &item.addr.endpoint {
            return Ok(endpoint.socket_addr());
        }
    }
    let mut msg = String::from(
        "discovery did not yield one NPC-server address; the NC socket was not opened",
    );
    if found.is_empty() {
        msg.push_str(". No id 79 reply arrived");
    }
    for item in found {
        msg.push_str(&format!(
            "\n{}\t{}\t{}",
            item.player_id, item.nickname, item.addr.raw
        ));
    }
    Err(msg)
}

fn connect_nc(o: &Opts, c: &Common) -> Result<NcSession, String> {
    let addr = nc_address(o, c)?;
    let mut session = NcSession::connect(&addr, &c.account, &c.password, c.timeout)
        .map_err(|e| format!("{addr}: NC login failed: {e}"))?;
    let peek = Duration::from_millis(200).min(c.timeout);
    session
        .peek(Instant::now() + peek)
        .map_err(|e| format!("{addr}: NC login failed: {e}"))?;
    eprintln!("connected to {addr} as {} (NPC Control)", c.account);
    Ok(session)
}

fn settle_nc(session: &mut NcSession, o: &Opts, c: &Common) -> Result<Vec<NcEvent>, String> {
    let quiet = Duration::from_millis(o.settle.unwrap_or(1000));
    let events = session
        .settle(quiet, Instant::now() + c.timeout)
        .map_err(|e| e.to_string())?;
    for event in &events {
        if let NcEvent::Disconnect(why) = event {
            return Err(format!("NPC-server disconnected: {why}"));
        }
        if o.verbose {
            eprintln!("{}", nc_verbose(event));
        }
    }
    Ok(events)
}

fn nc_verbose(event: &NcEvent) -> String {
    match event {
        NcEvent::Disconnect(why) => format!("[disconnected] {why}"),
        NcEvent::Message { id, payload } => format!("[nc] id {id} ({} bytes)", payload.len()),
        NcEvent::ClassDeleted(name) => format!("[class] deleted {name}"),
        NcEvent::Npc(row) => format!("[npc] {} arrived", row.id),
        NcEvent::NpcRemoved(id) => format!("[npc] {id} removed"),
        NcEvent::Attributes(_) => "[npc] attributes".into(),
        NcEvent::NpcScript { id, .. } => format!("[npc] script {id}"),
        NcEvent::Class(name) => format!("[class] {name}"),
        NcEvent::ClassScript { name, .. } => format!("[class] script {name}"),
        NcEvent::Weapons(names) => format!("[weapons] {} name(s)", names.len()),
        NcEvent::Weapon(weapon) => format!("[weapon] {}", weapon.name),
    }
}

fn seen(events: &[NcEvent]) -> String {
    if events.is_empty() {
        return "no messages".into();
    }
    events
        .iter()
        .map(|event| match event {
            NcEvent::Message { id, payload } => format!("id {id} ({} bytes)", payload.len()),
            NcEvent::ClassDeleted(name) => format!("[class] deleted {name}"),
            other => nc_verbose(other),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn print_body(text: &str) {
    if text.is_empty() || text.ends_with('\n') {
        print!("{text}");
    } else {
        println!("{text}");
    }
}

fn load_script(o: &Opts) -> Result<String, String> {
    match (&o.script, &o.script_file) {
        (Some(_), Some(_)) => Err("give either --script or --script-file".into()),
        (Some(text), None) => Ok(text.clone()),
        (None, Some(path)) => std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}")),
        (None, None) => Err("--script TEXT or --script-file PATH is required".into()),
    }
}

fn parse_npc_id(text: &str) -> Result<u32, String> {
    let id: u32 = text
        .parse()
        .map_err(|_| format!("NPC id: not a number: {text}"))?;
    if id > seihai_rc::nc::GINT3_MAX {
        return Err("NPC id does not fit in a GINT3".into());
    }
    Ok(id)
}

fn cmd_nc_discover(o: &Opts) -> Result<(), String> {
    if o.discover.is_some() {
        return Err("nc discover takes --server as the game server, not --discover".into());
    }
    let c = common(o)?;
    let game = o
        .server
        .clone()
        .ok_or("nc discover needs --server HOST:PORT (the game server)")?;
    let found = discover_at(&game, o, &c)?;
    if found.is_empty() {
        return Err("no NPC-server address (id 79) arrived".into());
    }
    println!("player\tnickname\taddress\tendpoint");
    for item in &found {
        let endpoint = item
            .addr
            .endpoint
            .as_ref()
            .map(|endpoint| endpoint.socket_addr())
            .unwrap_or_default();
        println!(
            "{}\t{}\t{}\t{}",
            item.player_id, item.nickname, item.addr.raw, endpoint
        );
    }
    Ok(())
}

fn cmd_nc_npcs(o: &Opts) -> Result<(), String> {
    let c = common(o)?;
    let mut session = connect_nc(o, &c)?;
    settle_nc(&mut session, o, &c)?;
    if o.tsv {
        println!("id\tname\ttype\tlevel");
    } else {
        println!("{:>8}  {:<20}  {:<12}  level", "id", "name", "type");
    }
    for row in session.lists.npcs.values() {
        let name = row.name.as_deref().unwrap_or("");
        let kind = row.kind.as_deref().unwrap_or("");
        let level = row.level.as_deref().unwrap_or("");
        if o.tsv {
            println!("{}\t{}\t{}\t{}", row.id, name, kind, level);
        } else {
            println!("{:>8}  {:<20}  {:<12}  {}", row.id, name, kind, level);
        }
    }
    if !o.tsv {
        println!("{} NPC(s)", session.lists.npcs.len());
    }
    session.close();
    Ok(())
}

fn cmd_nc_npc_get(o: &Opts, id_text: &str) -> Result<(), String> {
    let id = parse_npc_id(id_text)?;
    let c = common(o)?;
    let mut session = connect_nc(o, &c)?;
    let read_script = o.script_flag || o.script.is_some();
    let msg = if read_script {
        npc_script_get_message(id)
    } else {
        npc_get_message(id)
    }
    .map_err(|e| e.to_string())?;
    session.send(&[msg]).map_err(|e| e.to_string())?;
    let events = settle_nc(&mut session, o, &c)?;
    if read_script {
        let script = events.iter().find_map(|event| match event {
            NcEvent::NpcScript { id: got, script } if *got == id => Some(script.as_str()),
            _ => None,
        });
        match script {
            Some(script) => print_body(script),
            None => {
                return Err(format!(
                    "no NPC script (id 160) for {id} arrived; saw {}",
                    seen(&events)
                ));
            }
        }
    } else {
        let text = events.iter().find_map(|event| match event {
            NcEvent::Attributes(text) => Some(text.as_str()),
            _ => None,
        });
        match text {
            Some(text) => print_body(text),
            None => {
                return Err(format!(
                    "no NPC attributes (id 157) arrived after NC_NPCGET; a different reply id still needs a lab check. Saw {}",
                    seen(&events)
                ));
            }
        }
    }
    session.close();
    Ok(())
}

fn cmd_nc_npc_set(o: &Opts, id_text: &str) -> Result<(), String> {
    let id = parse_npc_id(id_text)?;
    let script = load_script(o)?;
    let msg = npc_script_set_message(id, &script).map_err(|e| e.to_string())?;
    let c = common(o)?;
    let mut session = connect_nc(o, &c)?;
    session.send(&[msg]).map_err(|e| e.to_string())?;
    settle_nc(&mut session, o, &c)?;
    println!("sent NPC script for {id}");
    session.close();
    Ok(())
}

fn cmd_nc_npc_delete(o: &Opts, id_text: &str) -> Result<(), String> {
    let id = parse_npc_id(id_text)?;
    let c = common(o)?;
    let mut session = connect_nc(o, &c)?;
    let msg = npc_delete_message(id).map_err(|e| e.to_string())?;
    session.send(&[msg]).map_err(|e| e.to_string())?;
    let events = settle_nc(&mut session, o, &c)?;
    if events
        .iter()
        .any(|event| matches!(event, NcEvent::NpcRemoved(got) if *got == id))
    {
        println!("deleted {id}");
        session.close();
        Ok(())
    } else {
        Err(format!(
            "no NPC delete (id 159) for {id} arrived; saw {}",
            seen(&events)
        ))
    }
}

fn cmd_nc_classes(o: &Opts) -> Result<(), String> {
    let c = common(o)?;
    let mut session = connect_nc(o, &c)?;
    settle_nc(&mut session, o, &c)?;
    for name in &session.lists.classes {
        println!("{name}");
    }
    session.close();
    Ok(())
}

fn cmd_nc_class_get(o: &Opts, name: &str) -> Result<(), String> {
    let msg = class_get_message(name).map_err(|e| e.to_string())?;
    let c = common(o)?;
    let mut session = connect_nc(o, &c)?;
    session.send(&[msg]).map_err(|e| e.to_string())?;
    let events = settle_nc(&mut session, o, &c)?;
    let script = events.iter().find_map(|event| match event {
        NcEvent::ClassScript { name: got, script } if got == name => Some(script.as_str()),
        _ => None,
    });
    match script {
        Some(script) => print_body(script),
        None => {
            return Err(format!(
                "no class script (id 162) for {name:?} arrived; saw {}",
                seen(&events)
            ));
        }
    }
    session.close();
    Ok(())
}

fn cmd_nc_class_set(o: &Opts, name: &str) -> Result<(), String> {
    let script = load_script(o)?;
    let msg = class_set_message(name, &script).map_err(|e| e.to_string())?;
    let c = common(o)?;
    let mut session = connect_nc(o, &c)?;
    session.send(&[msg]).map_err(|e| e.to_string())?;
    settle_nc(&mut session, o, &c)?;
    println!("sent class {name}");
    session.close();
    Ok(())
}

fn cmd_nc_class_delete(o: &Opts, name: &str) -> Result<(), String> {
    let msg = class_delete_message(name).map_err(|e| e.to_string())?;
    let c = common(o)?;
    let mut session = connect_nc(o, &c)?;
    session.send(&[msg]).map_err(|e| e.to_string())?;
    let events = settle_nc(&mut session, o, &c)?;
    if events
        .iter()
        .any(|event| matches!(event, NcEvent::ClassDeleted(got) if got == name))
    {
        println!("deleted {name}");
        session.close();
        Ok(())
    } else {
        Err(format!(
            "no class delete (id 188) for {name:?} arrived; a bare class name is not read. Saw {}",
            seen(&events)
        ))
    }
}

fn cmd_nc_weapons(o: &Opts) -> Result<(), String> {
    let c = common(o)?;
    let mut session = connect_nc(o, &c)?;
    session
        .send(&[weapon_list_message()])
        .map_err(|e| e.to_string())?;
    let events = settle_nc(&mut session, o, &c)?;
    let names = events.iter().rev().find_map(|event| match event {
        NcEvent::Weapons(names) => Some(names),
        _ => None,
    });
    let Some(names) = names else {
        return Err(format!(
            "no weapon list (id 167) arrived; saw {}",
            seen(&events)
        ));
    };
    for name in names {
        println!("{name}");
    }
    session.close();
    Ok(())
}

fn cmd_nc_weapon_get(o: &Opts, name: &str) -> Result<(), String> {
    let msg = weapon_get_message(name).map_err(|e| e.to_string())?;
    let c = common(o)?;
    let mut session = connect_nc(o, &c)?;
    session.send(&[msg]).map_err(|e| e.to_string())?;
    let events = settle_nc(&mut session, o, &c)?;
    let weapon = events.iter().find_map(|event| match event {
        NcEvent::Weapon(weapon) if weapon.name == name => Some(weapon),
        _ => None,
    });
    let Some(weapon) = weapon else {
        return Err(format!(
            "no weapon script (id 33) for {name:?} arrived; a GINT3 length or a line feed inside the script still needs a lab check. Saw {}",
            seen(&events)
        ));
    };
    eprintln!("image: {}", weapon.image);
    print_body(&weapon.script);
    session.close();
    Ok(())
}

fn cmd_nc_weapon_set(o: &Opts, name: &str) -> Result<(), String> {
    let image = o.image.clone().ok_or("weapon set needs --image NAME")?;
    let script = load_script(o)?;
    let msg = weapon_set_message(name, &image, &script).map_err(|e| e.to_string())?;
    let c = common(o)?;
    let mut session = connect_nc(o, &c)?;
    session.send(&[msg]).map_err(|e| e.to_string())?;
    settle_nc(&mut session, o, &c)?;
    println!("sent weapon {name}");
    session.close();
    Ok(())
}

fn cmd_nc_weapon_delete(o: &Opts, name: &str) -> Result<(), String> {
    let msg = weapon_delete_message(name).map_err(|e| e.to_string())?;
    let c = common(o)?;
    let mut session = connect_nc(o, &c)?;
    session.send(&[msg]).map_err(|e| e.to_string())?;
    settle_nc(&mut session, o, &c)?;
    println!("sent weapon delete for {name}");
    session.close();
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("seihai-rc {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{USAGE}");
        return if args.is_empty() {
            ExitCode::from(2)
        } else {
            ExitCode::SUCCESS
        };
    }
    let o = match parse_args(args.into_iter()) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("seihai-rc: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let cmd: Vec<&str> = o.cmd.iter().map(String::as_str).collect();
    let result = match cmd.as_slice() {
        ["servers"] => cmd_servers(&o),
        ["login"] => cmd_login(&o),
        ["players", "list"] | ["players"] => cmd_players_list(&o),
        ["players", "pm", who, words @ ..] if !words.is_empty() => cmd_players_pm(&o, who, words),
        ["players", "say", words @ ..] if !words.is_empty() => cmd_players_say(&o, words),
        ["players", "admin-pm", who, words @ ..] if !words.is_empty() => {
            cmd_players_admin_pm(&o, who, words)
        }
        ["players", "comments", account] => cmd_players_comments(&o, account),
        ["players", "ban", account] => cmd_players_ban(&o, account),
        ["players", "rights", account] => cmd_players_rights(&o, account),
        ["players", "kick", who] => cmd_players_kick(&o, who),
        ["players", "warp", who, level, x, y] => cmd_players_warp(&o, who, level, x, y),
        ["nc", "login"] => cmd_nc_login(&o),
        ["nc", "discover"] => cmd_nc_discover(&o),
        ["nc", "npcs"] | ["nc", "npc", "list"] => cmd_nc_npcs(&o),
        ["nc", "npc", "get", id] => cmd_nc_npc_get(&o, id),
        ["nc", "npc", "set", id] => cmd_nc_npc_set(&o, id),
        ["nc", "npc", "delete", id] => cmd_nc_npc_delete(&o, id),
        ["nc", "classes"] | ["nc", "class", "list"] => cmd_nc_classes(&o),
        ["nc", "class", "get", name] => cmd_nc_class_get(&o, name),
        ["nc", "class", "set", name] => cmd_nc_class_set(&o, name),
        ["nc", "class", "delete", name] => cmd_nc_class_delete(&o, name),
        ["nc", "weapons"] | ["nc", "weapon", "list"] => cmd_nc_weapons(&o),
        ["nc", "weapon", "get", name] => cmd_nc_weapon_get(&o, name),
        ["nc", "weapon", "set", name] => cmd_nc_weapon_set(&o, name),
        ["nc", "weapon", "delete", name] => cmd_nc_weapon_delete(&o, name),
        _ => {
            eprintln!(
                "seihai-rc: unknown command {:?}\n\n{USAGE}",
                o.cmd.join(" ")
            );
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("seihai-rc: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> impl Iterator<Item = String> + '_ {
        s.split_whitespace().map(String::from)
    }

    #[test]
    fn parses_players_list() {
        let o = parse_args(args(
            "players list --server 127.0.0.1:14802 --account a --tsv --settle 200",
        ))
        .unwrap();
        assert_eq!(o.cmd, ["players", "list"]);
        assert_eq!(o.server.as_deref(), Some("127.0.0.1:14802"));
        assert!(o.tsv);
        assert_eq!(o.settle, Some(200));
    }

    #[test]
    fn parses_players_actions() {
        let o = parse_args(args(
            "players warp alice cave.nw 30.5 12 --server 127.0.0.1:14802",
        ))
        .unwrap();
        assert_eq!(o.cmd, ["players", "warp", "alice", "cave.nw", "30.5", "12"]);
        let o = parse_args(args(
            "players pm 2,bob hello there --server 127.0.0.1:14802",
        ))
        .unwrap();
        assert_eq!(o.cmd, ["players", "pm", "2,bob", "hello", "there"]);
        assert_eq!(parse_tiles("X", "30.5"), Ok(30.5));
        assert!(parse_tiles("X", "east").is_err());
    }

    #[test]
    fn rights_bits_are_listed() {
        assert_eq!(bit_list(513), "0, 9");
        assert_eq!(bit_list(0), "none");
    }

    #[test]
    fn rejects_unknown_option() {
        assert!(parse_args(args("login --password secret")).is_err());
    }

    #[test]
    fn login_lines_distinguish_arrival_and_update() {
        let player = PlayerEntry {
            id: 6,
            account: "test205".into(),
            level: "cave.nw".into(),
            ..Default::default()
        };
        assert_eq!(
            player_line("online", &player),
            "[online] 6 test205 (cave.nw)"
        );
        assert_eq!(
            player_line("update", &player),
            "[update] 6 test205 (cave.nw)"
        );
        let rc = PlayerEntry {
            id: 5,
            account: "labrc2".into(),
            level: " ".into(),
            ..Default::default()
        };
        assert_eq!(player_line("online", &rc), "[online] 5 labrc2 (RC)");
        assert_eq!(player_line("update", &rc), "[update] 5 labrc2 (RC)");
    }

    #[test]
    fn parses_nc_login() {
        let o = parse_args(args(
            "nc login --server 127.0.0.1:14804 --account staff --for 0",
        ))
        .unwrap();
        assert_eq!(o.cmd, ["nc", "login"]);
        assert_eq!(o.server.as_deref(), Some("127.0.0.1:14804"));
        assert_eq!(o.account.as_deref(), Some("staff"));
        assert_eq!(o.for_secs, Some(0));
    }

    #[test]
    fn parses_nc_npc_set_and_script_flag() {
        let o = parse_args(args(
            "nc npc set 12 --discover 127.0.0.1:14802 --account staff --script return;",
        ))
        .unwrap();
        assert_eq!(o.cmd, ["nc", "npc", "set", "12"]);
        assert_eq!(o.discover.as_deref(), Some("127.0.0.1:14802"));
        assert_eq!(o.script.as_deref(), Some("return;"));
        let get = parse_args(args("nc npc get 12 --script --server 127.0.0.1:14804")).unwrap();
        assert!(get.script_flag);
        assert!(get.script.is_none());
    }
}
