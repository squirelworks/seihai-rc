//! `seihai-rc`: the command line.

use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use seihai_rc::accounts::{
    account_add, account_delete, account_get, account_list, account_reset, account_set,
    parse_account, parse_account_list, plo as accounts_plo, Account,
};
use seihai_rc::files::{
    delete_message, folder_delete_message, move_message, rename_message, split_path, FileError,
};
use seihai_rc::listserver::{decode_server_key, fetch_rc_list, fetch_rc_list_secure, pick};
use seihai_rc::nc::{
    class_delete_message, class_get_message, class_set_message, npc_add_message,
    npc_delete_message, npc_flags_get_message, npc_flags_set_message, npc_get_message,
    npc_reset_message, npc_script_get_message, npc_script_set_message, npc_warp_message,
    weapon_delete_message, weapon_get_message, weapon_list_message, weapon_set_message, NcEvent,
    NcSession, NewNpc,
};
use seihai_rc::players::{
    admin_message, ban_get, ban_set, comments_get, comments_set, disconnect_player, parse_ban,
    parse_comments, parse_prop_edit, parse_props, parse_rights, plo as players_plo,
    private_admin_message, private_message, prop_name, props_get, props_get_account, props_set,
    resolve, rights_get, rights_set, warp_player, PropsReply, Rights,
};
use seihai_rc::rc::{
    discover_npc_servers, Event, LoginParams, NpcServerFound, PlayerEntry, RcKind, Session,
};
use seihai_rc::server::{
    flags_get, flags_set, normalized, parse_flags, parse_text, plo as server_plo, ServerText,
};

const USAGE: &str = "\
seihai-rc: Graal Remote Control client (stage 1, player actions, NPC Control)

USAGE:
  seihai-rc servers --listserver HOST:PORT --account NAME [options]
  seihai-rc login   (--server HOST:PORT | --listserver HOST:PORT --pick NAME|N)
                    --account NAME [--for SECS] [--say TEXT] [options]
  seihai-rc players list (--server ... | --listserver ... --pick ...)
                    --account NAME [--tsv | --json] [--settle MS] [options]
  seihai-rc rcs list (--server ... | --listserver ... --pick ...)
                    --account NAME [--tsv | --json] [--settle MS] [options]
  seihai-rc players pm WHO[,WHO...] TEXT... (--server ... | --listserver ... --pick ...)
  seihai-rc players admin-pm WHO TEXT... (--server ... | --listserver ... --pick ...)
  seihai-rc players say TEXT... (--server ... | --listserver ... --pick ...)
  seihai-rc players comments ACCOUNT [--set TEXT | --set-file PATH] (--server ... | ...)
  seihai-rc players ban ACCOUNT [--set on|off [--reason TEXT]] (--server ... | ...)
  seihai-rc players rights ACCOUNT [--set N] [--ip-range R] [--set-file PATH] (--server ...)
  seihai-rc players props WHO [--set NAME=VALUE,...] (--server ... | --listserver ... --pick ...)
  seihai-rc players kick WHO (--server ... | --listserver ... --pick ...)
  seihai-rc players warp WHO LEVEL X Y (--server ... | --listserver ... --pick ...)
  seihai-rc accounts list [PATTERN] [--where COND] (--server ... | --listserver ... --pick ...)
  seihai-rc accounts get ACCOUNT (--server ... | --listserver ... --pick ...)
  seihai-rc accounts set ACCOUNT [FIELDS] (--server ... | --listserver ... --pick ...)
  seihai-rc accounts add ACCOUNT --new-password-env VAR [FIELDS] (--server ...)
  seihai-rc accounts delete ACCOUNT --yes (--server ... | --listserver ... --pick ...)
  seihai-rc accounts reset ACCOUNT --yes (--server ... | --listserver ... --pick ...)
  seihai-rc server options [--set-file PATH] (--server ... | --listserver ... --pick ...)
  seihai-rc server folders [--set-file PATH] (--server ... | ...)
  seihai-rc server flags [--set-file PATH] (--server ... | ...)
  seihai-rc files ls [DIR] (--server ... | --listserver ... --pick ...)
  seihai-rc files get PATH [--out FILE] (--server ... | ...)
  seihai-rc files put LOCAL PATH (--server ... | ...)
  seihai-rc files mv PATH FOLDER (--server ... | ...)
  seihai-rc files rename PATH NEWNAME (--server ... | ...)
  seihai-rc files rm PATH --yes (--server ... | ...)
  seihai-rc files rmdir FOLDER --yes (--server ... | ...)
  seihai-rc nc discover --server HOST:PORT --account NAME [--settle MS] [options]
  seihai-rc nc login --server HOST:PORT --account NAME [--for SECS] [options]
  seihai-rc nc npcs (--server HOST:PORT | --discover HOST:PORT) --account NAME
  seihai-rc nc npc get ID [--script] (--server ... | --discover ...) --account NAME
  seihai-rc nc npc set ID --script TEXT (--server ... | --discover ...) --account NAME
  seihai-rc nc npc delete ID (--server ... | --discover ...) --account NAME
  seihai-rc nc npc add NAME ID TYPE LEVEL X Y (--server ... | --discover ...) --account NAME
  seihai-rc nc npc reset ID (--server ... | --discover ...) --account NAME
  seihai-rc nc npc warp ID LEVEL X Y (--server ... | --discover ...) --account NAME
  seihai-rc nc npc flags ID [--set-file PATH] (--server ... | --discover ...) --account NAME
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
  rcs list      Log in and print the other RCs online. They come from
                the player list (an RC has an empty level); no RC sends
                id 65, so neither does this.
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
                it (id 88) and reads it back.
  players rights
                Print an account's rights number and set bits, IP range
                and folder rights (id 83, reply 62). --set N, --ip-range
                and --set-file (folder rights, one per line) change those
                fields, keep the rest, send id 84 and read them back.
                Bit 0 is warp-to-xy and bit 9 admin message.
  players props Print a player's props, flags and weapons (id 73, reply
                72). WHO that is not online is read by account (id 74).
                --set changes props (id 76) and reads them back; NAME is
                max-hearts, hearts (halves allowed), gralats, arrows,
                bombs, glove or ap. The rest is sent back as read.
  players kick  Disconnect a player (id 61) and wait for it to leave (56).
  players warp  Warp a player (id 82) to LEVEL at X Y, in tiles; halves
                such as 30.5 are allowed.
                WHO is a player id or an account name; the client reads the
                player list first (--settle) and refuses a WHO not in it.
  accounts list Print the accounts whose name matches PATTERN (default
                %, which matches any) and the server-side --where
                condition, such as \"adminlevel>0\" (id 72, reply 70).
  accounts get  Print an account's e-mail, banned, load only, admin level,
                admin worlds and ban reason (id 77, reply 73).
  accounts set  Change the fields given and send the rest back as read
                (id 78), then read the account back and name any field
                the server did not keep. FIELDS: --email E, --banned on|off,
                --load-only on|off, --admin-level N, --admin-worlds W,
                --reason TEXT (ban reason, one item per line), and
                --new-password-env VAR (the new password; without it the
                password is left as it is). The ban time is not sent; the
                real RC sends none.
  accounts add  Make an account (id 70, the same fields), then read it
                back. --new-password-env is required.
  accounts delete
                Delete an account (id 71), then list it to check it is
                gone. Needs --yes, as it cannot be undone.
  accounts reset
                Reset an account's attributes to the server's default
                account (id 75); the server keeps its rights and folders.
                Waits for the server's RC chat line. Needs --yes.
  files ls      List the files in DIR (id 90, reply 66). Default: the
                folder the server starts in.
  files get     Fetch PATH (id 92, file 102) to --out FILE, or to stdout.
                A large file (68, 84, pieces, 69) is joined.
  files put     Send LOCAL as PATH (id 93), then list to check it is there.
                Over 60 KiB it goes as a large file (155, pieces, 156).
                Files are up to 64 MiB.
  files mv      Move PATH into FOLDER (id 96), then list both folders.
  files rename  Rename PATH to NEWNAME in the same folder (id 98), then list.
  files rm      Delete PATH (id 97), then list. Needs --yes.
  files rmdir   Delete FOLDER (id 160), then check it no longer lists.
                Needs --yes.
                Each command lists PATH's folder first, as the server
                names a file by the folder from the last list. The layouts
                of 96, 97, 98 and 160 are not captured from a real RC yet;
                the server's lines are printed and the list is the check.
  server options
                Print the server options, one per line (id 51, reply 76).
                --set-file replaces them (id 52) and reads them back.
  server folders
                The folder config the same way (id 53, reply 77; set 54).
  server flags  The server flags, one per line (id 68, reply 61; set 69).
                Each --set prints the line counts before and after and
                fails if the server reads back something else. Each
                layout matches a real 2004 RC (lab 2026-11-13).
  nc discover   Log in to the game server, ask where each \"(Server)\" player
                is listening (ids 94 and 79), and print the address text.
                This does not open the NPC-Control socket.
  nc login      Log in to an NPC-server (type 3, plain zlib) and print
                message ids for --for seconds.
  nc npcs       List NPC rows the NPC-server sends (id 158).
  nc npc        get reads attributes (id 103, reply 157). get --script reads
                the script (id 106, reply 160). set writes the script (id 109,
                tokenized, so it may hold line breaks). delete removes the
                NPC (id 104) and waits for id 159. add creates a database
                NPC (id 111) and waits for its id 158; the scripter is
                --account. reset (id 105) waits for id 29. warp moves it
                (id 107; no reply). flags prints its flags (id 108, reply
                161); --set-file replaces them (id 110), one per line, and
                reads them back.
  nc classes    List class names the NPC-server sends (id 163).
  nc class      get opens a class (id 112, reply 162). set saves it (id 113).
                delete sends id 119 with the bare name and waits for id 188.
  nc weapons    Ask for the weapon list (id 115, reply 167).
  nc weapon     get opens a weapon (id 116, reply 33; 192 to an NCL21075
                client). set saves it (id 117, lines joined by 0xA7).
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
  --nick NAME          Nickname to send after the login (default
                       *ACCOUNT, as real RCs always send one).
  --no-nick            Send no nickname after the login.
  --identity TEXT      gen5 only: text sent after the password in the
                       login (off by default; old RCs send none).
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
  --out FILE           files get: write the file here (default stdout).
  --tsv                players list, rcs list and nc npcs: tab-separated, with a header.
  --json               players list and rcs list: one JSON array.
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
    no_nick: bool,
    identity: Option<String>,
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
    email: Option<String>,
    banned: Option<String>,
    load_only: Option<String>,
    admin_level: Option<String>,
    admin_worlds: Option<String>,
    where_: Option<String>,
    new_password_env: Option<String>,
    yes: bool,
    server_key: Option<String>,
    out: Option<String>,
    script_flag: bool,
    secure_login: bool,
    tsv: bool,
    json: bool,
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
            "--no-nick" => o.no_nick = true,
            "--identity" => o.identity = Some(val!(&a)),
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
            "--email" => o.email = Some(val!(&a)),
            "--banned" => o.banned = Some(val!(&a)),
            "--load-only" => o.load_only = Some(val!(&a)),
            "--admin-level" => o.admin_level = Some(val!(&a)),
            "--admin-worlds" => o.admin_worlds = Some(val!(&a)),
            "--where" => o.where_ = Some(val!(&a)),
            "--new-password-env" => o.new_password_env = Some(val!(&a)),
            "--yes" => o.yes = true,
            "--out" => o.out = Some(val!(&a)),
            "--server-key" => o.server_key = Some(val!(&a)),
            "--secure-login" => o.secure_login = true,
            "--timeout" => o.timeout = Some(num(&a, val!(&a))?),
            "--settle" => o.settle = Some(num(&a, val!(&a))?),
            "--for" => o.for_secs = Some(num(&a, val!(&a))?),
            "--tsv" => o.tsv = true,
            "--json" => o.json = true,
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
    if o.identity.is_some() && kind == RcKind::Classic {
        return Err("--identity is for the type-6 (gen5) login only".into());
    }
    if o.nick.is_some() && o.no_nick {
        return Err("give --nick or --no-nick, not both".into());
    }
    if o.json && o.tsv {
        return Err("give --json or --tsv, not both".into());
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
    let p = login_params(o, c);
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

fn hex_bytes(raw: &[u8]) -> String {
    raw.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
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
        Event::OtherProps(id, _) if verbose => eprintln!("[id 8] props for unlisted {id}"),
        Event::OtherProps(..) => {}
        Event::NpcServerAddr(addr) => {
            println!("[npc-server] {} {}", addr.player_id, addr.raw);
        }
        Event::PlayerFlags(raw) => println!("[player-flags] {}", hex_bytes(raw)),
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
        fetch_rc_list(
            ls,
            c.kind == RcKind::Classic,
            &c.account,
            &c.password,
            c.key,
            c.timeout,
        )
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

/// A JSON string, with the escapes JSON needs.
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `players list --json` and `rcs list --json`: one array of objects, the
/// same fields as `--tsv`.
fn players_json(entries: &[&PlayerEntry]) -> String {
    let rows: Vec<String> = entries
        .iter()
        .map(|e| {
            format!(
                "{{\"id\":{},\"kind\":{},\"account\":{},\"nickname\":{},\"level\":{},\"status\":{},\"community\":{}}}",
                e.id,
                json_str(if e.is_rc() { "rc" } else { "player" }),
                json_str(&e.account),
                json_str(&e.nickname),
                json_str(&e.level),
                e.status,
                json_str(&e.community)
            )
        })
        .collect();
    format!("[{}]", rows.join(","))
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
    if o.json {
        let all: Vec<&PlayerEntry> = s.players.players.values().collect();
        println!("{}", players_json(&all));
        s.close();
        return Ok(());
    }
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

/// The other RCs online: the player list filtered through `is_rc`. Real
/// RCs list RCs this way; id 65 is never sent.
fn cmd_rcs_list(o: &Opts) -> Result<(), String> {
    let (s, _) = players_session(o)?;
    if o.json {
        let rcs: Vec<&PlayerEntry> = s.players.players.values().filter(|e| e.is_rc()).collect();
        println!("{}", players_json(&rcs));
        s.close();
        return Ok(());
    }
    if o.tsv {
        println!("id\taccount\tnickname\tstatus\tcommunity");
    } else {
        println!("{:>5}  {:<20}  nickname", "id", "account");
    }
    let rcs: Vec<&PlayerEntry> = s.players.players.values().filter(|e| e.is_rc()).collect();
    for e in &rcs {
        if o.tsv {
            println!(
                "{}\t{}\t{}\t{}\t{}",
                e.id, e.account, e.nickname, e.status, e.community
            );
        } else {
            println!("{:>5}  {:<20}  {}", e.id, e.account, e.nickname);
        }
    }
    if !o.tsv {
        println!("{} other RC(s)", rcs.len());
    }
    s.close();
    Ok(())
}

fn cmd_files_ls(o: &Opts, dir: &str) -> Result<(), String> {
    let (mut s, c) = players_session(o)?;
    let names = s.list_dir(dir, c.timeout).map_err(|e| e.to_string())?;
    s.close();
    for n in names {
        println!("{n}");
    }
    Ok(())
}

fn cmd_files_get(o: &Opts, path: &str) -> Result<(), String> {
    let (folder, name) = split_path(path);
    let (mut s, c) = players_session(o)?;
    s.list_dir(folder, c.timeout).map_err(|e| e.to_string())?;
    let data = s.get_file(name, c.timeout).map_err(|e| e.to_string())?;
    s.close();
    match &o.out {
        Some(out) => {
            std::fs::write(out, &data).map_err(|e| format!("{out}: {e}"))?;
            eprintln!("wrote {} bytes to {out}", data.len());
        }
        None => {
            use std::io::Write;
            std::io::stdout()
                .write_all(&data)
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// List `folder` and say whether `name` is in it.
fn listed(s: &mut Session, c: &Common, folder: &str, name: &str) -> Result<bool, String> {
    let names = s.list_dir(folder, c.timeout).map_err(|e| e.to_string())?;
    Ok(names.iter().any(|n| n == name))
}

fn cmd_files_put(o: &Opts, local: &str, path: &str) -> Result<(), String> {
    let data = std::fs::read(local).map_err(|e| format!("{local}: {e}"))?;
    let (folder, name) = split_path(path);
    let (mut s, c) = players_session(o)?;
    s.list_dir(folder, c.timeout).map_err(|e| e.to_string())?;
    s.put_file(name, &data, c.timeout)
        .map_err(|e| e.to_string())?;
    let there = listed(&mut s, &c, folder, name)?;
    s.close();
    if !there {
        return Err(format!("the server answered, but {path} is not listed"));
    }
    println!("sent {} bytes as {path}", data.len());
    Ok(())
}

/// Send one file action and print the server's id 67 lines.
fn file_action(s: &mut Session, o: &Opts, c: &Common, msg: Vec<u8>) -> Result<(), String> {
    let quiet = Duration::from_millis(o.settle.unwrap_or(1000));
    let lines = s
        .file_action(msg, quiet, Instant::now() + c.timeout)
        .map_err(|e| e.to_string())?;
    for l in lines {
        println!("[server] {l}");
    }
    Ok(())
}

fn cmd_files_mv(o: &Opts, path: &str, to: &str) -> Result<(), String> {
    let (folder, name) = split_path(path);
    let msg = move_message(name, to).map_err(|e| e.to_string())?;
    let (mut s, c) = players_session(o)?;
    if !listed(&mut s, &c, folder, name)? {
        s.close();
        return Err(format!("{path} is not listed"));
    }
    file_action(&mut s, o, &c, msg)?;
    let still = listed(&mut s, &c, folder, name)?;
    let moved = listed(&mut s, &c, to, name)?;
    s.close();
    if still || !moved {
        return Err(format!(
            "{path} was not moved (in {folder:?}: {still}, in {to:?}: {moved})"
        ));
    }
    println!("moved {path} to {to}");
    Ok(())
}

fn cmd_files_rename(o: &Opts, path: &str, new: &str) -> Result<(), String> {
    let (folder, name) = split_path(path);
    let msg = rename_message(name, new).map_err(|e| e.to_string())?;
    let (mut s, c) = players_session(o)?;
    if !listed(&mut s, &c, folder, name)? {
        s.close();
        return Err(format!("{path} is not listed"));
    }
    file_action(&mut s, o, &c, msg)?;
    let names = s.list_dir(folder, c.timeout).map_err(|e| e.to_string())?;
    s.close();
    if names.iter().any(|n| n == name) || !names.iter().any(|n| n == new) {
        return Err(format!("{path} was not renamed to {new}"));
    }
    println!("renamed {path} to {folder}{new}");
    Ok(())
}

fn cmd_files_rm(o: &Opts, path: &str) -> Result<(), String> {
    if !o.yes {
        return Err("files rm cannot be undone; add --yes".into());
    }
    let (folder, name) = split_path(path);
    let msg = delete_message(name).map_err(|e| e.to_string())?;
    let (mut s, c) = players_session(o)?;
    if !listed(&mut s, &c, folder, name)? {
        s.close();
        return Err(format!("{path} is not listed"));
    }
    file_action(&mut s, o, &c, msg)?;
    let still = listed(&mut s, &c, folder, name)?;
    s.close();
    if still {
        return Err(format!("{path} is still listed"));
    }
    println!("deleted {path}");
    Ok(())
}

fn cmd_files_rmdir(o: &Opts, folder: &str) -> Result<(), String> {
    if !o.yes {
        return Err("files rmdir cannot be undone; add --yes".into());
    }
    let msg = folder_delete_message(folder).map_err(|e| e.to_string())?;
    let (mut s, c) = players_session(o)?;
    s.list_dir(folder, c.timeout).map_err(|e| e.to_string())?;
    file_action(&mut s, o, &c, msg)?;
    let after = s.list_dir(folder, c.timeout);
    s.close();
    match after {
        Err(FileError::Refused(why)) => {
            println!("deleted folder {folder} (it no longer lists: {why})");
            Ok(())
        }
        // The fork does not answer a list of a folder the account cannot open.
        Err(FileError::Timeout) => {
            println!("deleted folder {folder} (no answer when listed again)");
            Ok(())
        }
        Err(e) => Err(e.to_string()),
        Ok(_) => Err(format!("{folder} still lists after the delete")),
    }
}

/// Read events until message `id` arrives, then parse it.
fn await_id<T>(
    s: &mut Session,
    c: &Common,
    id: u8,
    parse: impl Fn(&[u8]) -> Option<T>,
) -> Result<T, String> {
    let deadline = Instant::now() + c.timeout;
    loop {
        match s.next_event(deadline).map_err(|e| e.to_string())? {
            Some(Event::Other(got, payload)) if got == id => {
                return parse(&payload).ok_or_else(|| format!("could not read the id {id} reply"));
            }
            Some(Event::Disconnect(why)) => {
                return Err(format!("the server closed the link: {why}"))
            }
            Some(_) => {}
            None => return Err(format!("no reply (id {id}) before the timeout")),
        }
    }
}

/// The new lines from `--set-file`, if given. `--set` is not taken: these
/// are many lines.
fn set_file_lines(o: &Opts) -> Result<Option<String>, String> {
    if o.set.is_some() {
        return Err("use --set-file PATH, one entry per line".into());
    }
    match &o.set_file {
        Some(path) => Ok(Some(
            std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?,
        )),
        None => Ok(None),
    }
}

/// Get, and with `--set-file` set and read back: options or folder config.
fn cmd_server_text(o: &Opts, which: ServerText) -> Result<(), String> {
    let new = set_file_lines(o)?;
    let (mut s, c) = players_session(o)?;
    let reply = which.ids().1;
    let get = |s: &mut Session| -> Result<Vec<String>, String> {
        s.send_messages(&[which.get_message()])
            .map_err(|e| e.to_string())?;
        await_id(s, &c, reply, |p| Some(parse_text(p)))
    };
    let before = get(&mut s)?;
    let Some(text) = new else {
        s.close();
        for l in &before {
            println!("{l}");
        }
        return Ok(());
    };
    let text = text.trim_end_matches(['\n', '\r']).to_string();
    s.send_messages(&[which.set_message(&text)])
        .map_err(|e| e.to_string())?;
    let after = get(&mut s)?;
    s.close();
    for l in &after {
        println!("{l}");
    }
    eprintln!("{} line(s) before, {} after", before.len(), after.len());
    let want: Vec<String> = text.lines().map(str::to_string).collect();
    if normalized(&after) != normalized(&want) {
        return Err("the server read back something other than what was sent".into());
    }
    Ok(())
}

/// Get, and with `--set-file` set and read back: the server flags.
fn cmd_server_flags(o: &Opts) -> Result<(), String> {
    let new = set_file_lines(o)?;
    let set = match &new {
        Some(text) => {
            let lines: Vec<String> = text.lines().map(|l| l.trim_end().to_string()).collect();
            Some((flags_set(&lines)?, lines))
        }
        None => None,
    };
    let (mut s, c) = players_session(o)?;
    let get = |s: &mut Session| -> Result<Vec<String>, String> {
        s.send_messages(&[flags_get()]).map_err(|e| e.to_string())?;
        await_id(s, &c, server_plo::RC_SERVERFLAGSGET, parse_flags)
    };
    let before = get(&mut s)?;
    let Some((msg, want)) = set else {
        s.close();
        for f in &before {
            println!("{f}");
        }
        return Ok(());
    };
    s.send_messages(&[msg]).map_err(|e| e.to_string())?;
    let after = get(&mut s)?;
    s.close();
    for f in &after {
        println!("{f}");
    }
    eprintln!("{} flag(s) before, {} after", before.len(), after.len());
    // The server may keep flags in its own order.
    let mut a = normalized(&after);
    let mut w = normalized(&want);
    a.sort();
    w.sort();
    if a != w {
        return Err("the server read back other flags than were sent".into());
    }
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

/// A prop value for people: text when the bytes are a GSTRING that fills
/// the value, a number for one byte or a GINT3, and escaped bytes otherwise.
fn prop_value(id: u8, raw: &[u8]) -> String {
    let n = |b: &u8| u32::from(b.wrapping_sub(0x20) & 0x7F);
    let gstring = raw.len() > 1 && usize::from(raw[0].wrapping_sub(0x20)) == raw.len() - 1;
    match raw {
        [b] => n(b).to_string(),
        [a, b, c] if !gstring || matches!(id, 3 | 27 | 28 | 29 | 36) => {
            ((n(a) << 14) | (n(b) << 7) | n(c)).to_string()
        }
        _ if gstring => seihai_rc::wire::text(&raw[1..]),
        _ => raw
            .iter()
            .map(|&b| {
                if (0x20..0x7f).contains(&b) {
                    (b as char).to_string()
                } else {
                    format!("\\x{b:02x}")
                }
            })
            .collect(),
    }
}

/// Ask for the props of one player (73, by id) or of an account that is
/// not online (74, by name), and wait for the 72 that answers it.
fn read_props(s: &mut Session, c: &Common, target: &PropsTarget) -> Result<PropsReply, String> {
    let (msg, id) = match target {
        PropsTarget::Online(id, _) => (props_get(*id).map_err(|e| e.to_string())?, *id),
        PropsTarget::Account(account) => {
            (props_get_account(account).map_err(|e| e.to_string())?, 0)
        }
    };
    s.send_messages(&[msg]).map_err(|e| e.to_string())?;
    await_reply(
        s,
        c,
        players_plo::RC_PLAYERPROPSGET,
        target.account(),
        |p| {
            parse_props(p)
                .filter(|r| r.id == id)
                .map(|r| (r.account.clone(), r))
        },
    )
}

enum PropsTarget {
    Online(u16, String),
    Account(String),
}

impl PropsTarget {
    fn account(&self) -> &str {
        match self {
            PropsTarget::Online(_, a) | PropsTarget::Account(a) => a,
        }
    }
}

fn print_props(r: &PropsReply) {
    if r.id == 0 {
        println!("account {} world {} (not online)", r.account, r.world);
    } else {
        println!("player {} account {} world {}", r.id, r.account, r.world);
    }
    for p in &r.props {
        println!(
            "{:>4}  {:<20}  {}",
            p.id,
            prop_name(p.id).unwrap_or("?"),
            prop_value(p.id, &p.raw)
        );
    }
    if !r.props_complete {
        println!("(the props block holds more than the prop table could read)");
    }
    println!("flags: {}", r.flags.len());
    for f in &r.flags {
        println!("  {f}");
    }
    println!("opened chests: {}", r.chests.len());
    for c in &r.chests {
        println!("  {},{} {}", c.x, c.y, c.level);
    }
    println!("weapons: {}", r.weapons.join(", "));
}

fn cmd_players_props(o: &Opts, who: &str) -> Result<(), String> {
    let edits = match &o.set {
        Some(list) => list
            .split(',')
            .filter(|i| !i.trim().is_empty())
            .map(parse_prop_edit)
            .collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
    };
    if o.set.is_some() && edits.is_empty() {
        return Err("--set needs NAME=VALUE".into());
    }
    let (mut s, c) = players_session(o)?;
    let digits = !who.is_empty() && who.bytes().all(|b| b.is_ascii_digit());
    let online = s
        .players
        .players
        .values()
        .any(|e| e.account.eq_ignore_ascii_case(who));
    let target = if digits || online {
        let e = resolve(&s.players, who)?.clone();
        if e.is_rc() {
            return Err(format!("{} {} is an RC, not a player", e.id, e.account));
        }
        PropsTarget::Online(e.id, e.account)
    } else {
        PropsTarget::Account(who.to_string())
    };
    let r = read_props(&mut s, &c, &target)?;
    if edits.is_empty() {
        s.close();
        print_props(&r);
        return Ok(());
    }
    let msg = props_set(&r, &edits)?;
    if r.flags.is_empty() {
        eprintln!(
            "note: the server sent no flags, so none go back; a server that stores flags it \
             does not send drops them on this set, as it does for the real RC"
        );
    }
    s.send_messages(&[msg]).map_err(|e| e.to_string())?;
    let back = read_props(&mut s, &c, &target)?;
    s.close();
    print_props(&back);
    let mut wrong = Vec::new();
    for (id, raw) in &edits {
        let got = back.props.iter().find(|p| p.id == *id).map(|p| &p.raw);
        if got != Some(raw) {
            wrong.push(prop_name(*id).unwrap_or("?"));
        }
    }
    if !wrong.is_empty() {
        return Err(format!(
            "sent the props for {}, but the server read back other values for: {}",
            r.account,
            wrong.join(", ")
        ));
    }
    eprintln!("set the props for {}", r.account);
    Ok(())
}

fn on_off(name: &str, v: &Option<String>) -> Result<Option<bool>, String> {
    match v.as_deref() {
        None => Ok(None),
        Some("on") => Ok(Some(true)),
        Some("off") => Ok(Some(false)),
        Some(other) => Err(format!("{name}: expected on or off, got {other}")),
    }
}

fn read_account(s: &mut Session, c: &Common, account: &str) -> Result<Account, String> {
    let msg = account_get(account).map_err(|e| e.to_string())?;
    s.send_messages(&[msg]).map_err(|e| e.to_string())?;
    await_reply(s, c, accounts_plo::RC_ACCOUNTGET, account, |p| {
        parse_account(p).map(|a| (a.account.clone(), a))
    })
}

fn read_account_list(
    s: &mut Session,
    c: &Common,
    pattern: &str,
    conditions: &str,
) -> Result<Vec<String>, String> {
    let msg = account_list(pattern, conditions).map_err(|e| e.to_string())?;
    s.send_messages(&[msg]).map_err(|e| e.to_string())?;
    await_reply(s, c, accounts_plo::RC_ACCOUNTLISTGET, pattern, |p| {
        parse_account_list(p).map(|l| (pattern.to_string(), l))
    })
}

fn print_account(a: &Account) {
    println!("account: {}", a.account);
    println!("e-mail: {}", a.email);
    println!("banned: {}", if a.banned { "yes" } else { "no" });
    println!("load only: {}", if a.load_only { "yes" } else { "no" });
    println!("admin level: {}", a.admin_level);
    println!("admin worlds: {}", a.admin_worlds);
    if !a.ban_length.is_empty() {
        println!("ban length: {}", a.ban_length);
    }
    for line in &a.ban_reason {
        println!("ban reason: {line}");
    }
}

/// Read and drop what the server sends for `--settle` ms (default 1000).
fn settle_quietly(s: &mut Session, o: &Opts, c: &Common) -> Result<(), String> {
    let quiet = Duration::from_millis(o.settle.unwrap_or(1000));
    s.settle(quiet, Instant::now() + c.timeout)
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn cmd_accounts_list(o: &Opts, pattern: &str) -> Result<(), String> {
    let conditions = o.where_.clone().unwrap_or_default();
    account_list(pattern, &conditions).map_err(|e| e.to_string())?;
    let c = common(o)?;
    let (mut s, _) = login(o, &c)?;
    let names = read_account_list(&mut s, &c, pattern, &conditions)?;
    s.close();
    for n in &names {
        println!("{n}");
    }
    Ok(())
}

fn cmd_accounts_get(o: &Opts, account: &str) -> Result<(), String> {
    account_get(account).map_err(|e| e.to_string())?;
    let c = common(o)?;
    let (mut s, _) = login(o, &c)?;
    let a = read_account(&mut s, &c, account)?;
    s.close();
    print_account(&a);
    Ok(())
}

/// Apply the field options to `a`. True when any was given.
fn apply_account_fields(o: &Opts, a: &mut Account) -> Result<bool, String> {
    let mut any = false;
    if let Some(e) = &o.email {
        a.email = e.clone();
        any = true;
    }
    if let Some(b) = on_off("--banned", &o.banned)? {
        a.banned = b;
        any = true;
    }
    if let Some(b) = on_off("--load-only", &o.load_only)? {
        a.load_only = b;
        any = true;
    }
    if let Some(n) = &o.admin_level {
        a.admin_level = n
            .parse()
            .map_err(|_| format!("--admin-level: not a level: {n}"))?;
        any = true;
    }
    if let Some(w) = &o.admin_worlds {
        a.admin_worlds = w.clone();
        any = true;
    }
    if let Some(r) = &o.reason {
        a.ban_reason = r.lines().map(str::to_string).collect();
        any = true;
    }
    Ok(any)
}

fn cmd_accounts_set(o: &Opts, account: &str, add: bool) -> Result<(), String> {
    let password = match &o.new_password_env {
        Some(var) => {
            let p = std::env::var(var).map_err(|_| format!("{var} is not set"))?;
            if p.is_empty() {
                return Err(format!("{var} is empty"));
            }
            p
        }
        None if add => return Err("accounts add needs --new-password-env".into()),
        None => String::new(),
    };
    // Check every field before logging in.
    // The RC's add dialog starts with admin worlds `all`; so does add.
    let mut probe = Account {
        account: account.to_string(),
        admin_worlds: if add { "all".into() } else { String::new() },
        ..Account::default()
    };
    let any = apply_account_fields(o, &mut probe)?;
    if !add && !any && password.is_empty() {
        return Err("accounts set: give at least one field to change".into());
    }
    account_set(&probe, &password).map_err(|e| e.to_string())?;
    let c = common(o)?;
    let (mut s, _) = login(o, &c)?;
    let want = if add {
        let a = probe;
        let msg = account_add(&a, &password).map_err(|e| e.to_string())?;
        s.send_messages(&[msg]).map_err(|e| e.to_string())?;
        // The fork answers a get or a list sent at once with what it had
        // before the add, so wait --settle ms first.
        settle_quietly(&mut s, o, &c)?;
        a
    } else {
        let mut a = read_account(&mut s, &c, account)?;
        apply_account_fields(o, &mut a)?;
        let msg = account_set(&a, &password).map_err(|e| e.to_string())?;
        s.send_messages(&[msg]).map_err(|e| e.to_string())?;
        a
    };
    let back = read_account(&mut s, &c, account)?;
    s.close();
    print_account(&back);
    let mut lost = Vec::new();
    if back.email != want.email {
        lost.push("e-mail");
    }
    if back.banned != want.banned {
        lost.push("banned");
    }
    if back.load_only != want.load_only {
        lost.push("load only");
    }
    if back.admin_level != want.admin_level {
        lost.push("admin level");
    }
    if back.admin_worlds != want.admin_worlds {
        lost.push("admin worlds");
    }
    if back.ban_reason != want.ban_reason {
        lost.push("ban reason");
    }
    let what = if add { "added" } else { "sent the changes for" };
    if !lost.is_empty() {
        return Err(format!(
            "{what} {account}, but the server read back other values for: {}",
            lost.join(", ")
        ));
    }
    eprintln!("{} {account}", if add { "added" } else { "set" });
    Ok(())
}

fn cmd_accounts_delete(o: &Opts, account: &str) -> Result<(), String> {
    let msg = account_delete(account).map_err(|e| e.to_string())?;
    if !o.yes {
        return Err(format!(
            "deleting {account} cannot be undone; add --yes to delete it"
        ));
    }
    let c = common(o)?;
    let (mut s, _) = login(o, &c)?;
    s.send_messages(&[msg]).map_err(|e| e.to_string())?;
    // As for add: a list sent at once still has the account.
    settle_quietly(&mut s, o, &c)?;
    let left = read_account_list(&mut s, &c, account, "")?;
    s.close();
    if left.iter().any(|n| n.eq_ignore_ascii_case(account)) {
        return Err(format!(
            "sent the delete for {account}, but the server still lists it"
        ));
    }
    println!("deleted {account}");
    Ok(())
}

fn cmd_accounts_reset(o: &Opts, account: &str) -> Result<(), String> {
    let msg = account_reset(account).map_err(|e| e.to_string())?;
    if !o.yes {
        return Err(format!(
            "resetting {account} cannot be undone; add --yes to reset it"
        ));
    }
    let c = common(o)?;
    let (mut s, _) = login(o, &c)?;
    let events = send_and_settle(&mut s, o, &c, msg)?;
    s.close();
    let said = events.iter().any(|e| {
        matches!(e, Event::RcChat(line)
            if line.contains("reset the attributes of account")
                && line.to_ascii_lowercase().ends_with(&account.to_ascii_lowercase()))
    });
    if !said {
        return Err(format!(
            "sent the reset for {account}, but the server did not confirm it"
        ));
    }
    println!("reset {account}");
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
        nickname: default_nick(o, &c.account),
        identity: o.identity.clone(),
    }
}

/// The nickname to send after the login. Real RCs always send one, so with
/// no `--nick` we send `*account`, the name the fork shows for an RC that
/// sends none: the same name on screen, and the same messages on the wire
/// as a real RC. `--no-nick` sends nothing.
fn default_nick(o: &Opts, account: &str) -> Option<String> {
    if o.no_nick {
        return None;
    }
    Some(o.nick.clone().unwrap_or_else(|| format!("*{account}")))
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
        NcEvent::NpcFlags { id, flags } => format!("[npc] {id} flags ({})", flags.len()),
        NcEvent::NpcReset(id) => format!("[npc] {id} reset"),
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

fn cmd_nc_npc_add(
    o: &Opts,
    name: &str,
    id_text: &str,
    kind: &str,
    level: &str,
    x: &str,
    y: &str,
) -> Result<(), String> {
    let id = parse_npc_id(id_text)?;
    let c = common(o)?;
    let msg = npc_add_message(&NewNpc {
        name: name.to_string(),
        id,
        kind: kind.to_string(),
        scripter: c.account.clone(),
        level: level.to_string(),
        x: x.to_string(),
        y: y.to_string(),
    })
    .map_err(|e| e.to_string())?;
    let mut session = connect_nc(o, &c)?;
    session.send(&[msg]).map_err(|e| e.to_string())?;
    let events = settle_nc(&mut session, o, &c)?;
    if events
        .iter()
        .any(|event| matches!(event, NcEvent::Npc(row) if row.id == id))
    {
        println!("added {id}");
        session.close();
        Ok(())
    } else {
        Err(format!(
            "no NPC row (id 158) for {id} arrived; saw {}",
            seen(&events)
        ))
    }
}

fn cmd_nc_npc_reset(o: &Opts, id_text: &str) -> Result<(), String> {
    let id = parse_npc_id(id_text)?;
    let msg = npc_reset_message(id).map_err(|e| e.to_string())?;
    let c = common(o)?;
    let mut session = connect_nc(o, &c)?;
    session.send(&[msg]).map_err(|e| e.to_string())?;
    let events = settle_nc(&mut session, o, &c)?;
    if events
        .iter()
        .any(|event| matches!(event, NcEvent::NpcReset(got) if *got == id))
    {
        println!("reset {id}");
        session.close();
        Ok(())
    } else {
        Err(format!(
            "no NPC reset (id 29) for {id} arrived; saw {}",
            seen(&events)
        ))
    }
}

fn cmd_nc_npc_warp(o: &Opts, id_text: &str, level: &str, x: &str, y: &str) -> Result<(), String> {
    let id = parse_npc_id(id_text)?;
    let x: f32 = x.parse().map_err(|_| format!("x: not a number: {x}"))?;
    let y: f32 = y.parse().map_err(|_| format!("y: not a number: {y}"))?;
    let msg = npc_warp_message(id, x, y, level).map_err(|e| e.to_string())?;
    let c = common(o)?;
    let mut session = connect_nc(o, &c)?;
    session.send(&[msg]).map_err(|e| e.to_string())?;
    settle_nc(&mut session, o, &c)?;
    println!("sent warp for {id}");
    session.close();
    Ok(())
}

/// The flags in the first 161 for `id`.
fn npc_flags_of(events: &[NcEvent], id: u32) -> Option<Vec<String>> {
    events.iter().find_map(|event| match event {
        NcEvent::NpcFlags { id: got, flags } if *got == id => Some(flags.clone()),
        _ => None,
    })
}

fn cmd_nc_npc_flags(o: &Opts, id_text: &str) -> Result<(), String> {
    let id = parse_npc_id(id_text)?;
    let new: Option<Vec<String>> = set_file_lines(o)?.map(|text| {
        text.lines()
            .map(str::to_string)
            .filter(|l| !l.is_empty())
            .collect()
    });
    let set_msg = match &new {
        Some(flags) => Some(npc_flags_set_message(id, flags).map_err(|e| e.to_string())?),
        None => None,
    };
    let get_msg = npc_flags_get_message(id).map_err(|e| e.to_string())?;
    let c = common(o)?;
    let mut session = connect_nc(o, &c)?;
    session
        .send(std::slice::from_ref(&get_msg))
        .map_err(|e| e.to_string())?;
    let events = settle_nc(&mut session, o, &c)?;
    let before = npc_flags_of(&events, id).ok_or_else(|| {
        format!(
            "no NPC flags (id 161) for {id} arrived; saw {}",
            seen(&events)
        )
    })?;
    let Some(set_msg) = set_msg else {
        for f in &before {
            println!("{f}");
        }
        session.close();
        return Ok(());
    };
    session
        .send(&[set_msg, get_msg])
        .map_err(|e| e.to_string())?;
    let events = settle_nc(&mut session, o, &c)?;
    session.close();
    let after = npc_flags_of(&events, id).ok_or_else(|| {
        format!(
            "no NPC flags (id 161) for {id} came back after the set; saw {}",
            seen(&events)
        )
    })?;
    println!("flags: {} before, {} after", before.len(), after.len());
    let want = new.unwrap_or_default();
    let mut got = after.clone();
    got.sort();
    let mut sorted_want = want.clone();
    sorted_want.sort();
    if got != sorted_want {
        return Err(format!("the server read back {after:?}, not {want:?}"));
    }
    Ok(())
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
            "no class delete (id 188) for {name:?} arrived; the server answers it only when the class existed. Saw {}",
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
        ["rcs", "list"] | ["rcs"] => cmd_rcs_list(&o),
        ["players", "pm", who, words @ ..] if !words.is_empty() => cmd_players_pm(&o, who, words),
        ["players", "say", words @ ..] if !words.is_empty() => cmd_players_say(&o, words),
        ["players", "admin-pm", who, words @ ..] if !words.is_empty() => {
            cmd_players_admin_pm(&o, who, words)
        }
        ["players", "comments", account] => cmd_players_comments(&o, account),
        ["players", "ban", account] => cmd_players_ban(&o, account),
        ["players", "rights", account] => cmd_players_rights(&o, account),
        ["players", "props", who] => cmd_players_props(&o, who),
        ["accounts", "list"] => cmd_accounts_list(&o, "%"),
        ["accounts", "list", pattern] => cmd_accounts_list(&o, pattern),
        ["accounts", "get", account] => cmd_accounts_get(&o, account),
        ["accounts", "set", account] => cmd_accounts_set(&o, account, false),
        ["accounts", "add", account] => cmd_accounts_set(&o, account, true),
        ["accounts", "delete", account] => cmd_accounts_delete(&o, account),
        ["accounts", "reset", account] => cmd_accounts_reset(&o, account),
        ["players", "kick", who] => cmd_players_kick(&o, who),
        ["players", "warp", who, level, x, y] => cmd_players_warp(&o, who, level, x, y),
        ["server", "options"] => cmd_server_text(&o, ServerText::Options),
        ["server", "folders"] => cmd_server_text(&o, ServerText::Folders),
        ["server", "flags"] => cmd_server_flags(&o),
        ["files", "ls"] => cmd_files_ls(&o, ""),
        ["files", "ls", dir] => cmd_files_ls(&o, dir),
        ["files", "get", path] => cmd_files_get(&o, path),
        ["files", "put", local, path] => cmd_files_put(&o, local, path),
        ["files", "mv", path, folder] => cmd_files_mv(&o, path, folder),
        ["files", "rename", path, name] => cmd_files_rename(&o, path, name),
        ["files", "rm", path] => cmd_files_rm(&o, path),
        ["files", "rmdir", folder] => cmd_files_rmdir(&o, folder),
        ["nc", "login"] => cmd_nc_login(&o),
        ["nc", "discover"] => cmd_nc_discover(&o),
        ["nc", "npcs"] | ["nc", "npc", "list"] => cmd_nc_npcs(&o),
        ["nc", "npc", "get", id] => cmd_nc_npc_get(&o, id),
        ["nc", "npc", "set", id] => cmd_nc_npc_set(&o, id),
        ["nc", "npc", "delete", id] => cmd_nc_npc_delete(&o, id),
        ["nc", "npc", "add", name, id, kind, level, x, y] => {
            cmd_nc_npc_add(&o, name, id, kind, level, x, y)
        }
        ["nc", "npc", "reset", id] => cmd_nc_npc_reset(&o, id),
        ["nc", "npc", "warp", id, level, x, y] => cmd_nc_npc_warp(&o, id, level, x, y),
        ["nc", "npc", "flags", id] => cmd_nc_npc_flags(&o, id),
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
    fn prop_values_read_as_text_or_numbers() {
        assert_eq!(prop_value(0, b"(Lab222rc"), "Lab222rc");
        assert_eq!(prop_value(3, b"  '"), "7");
        assert_eq!(prop_value(29, &[0x20, 0x23, 0x7b]), "475");
        assert_eq!(prop_value(1, b"#"), "3");
        assert_eq!(prop_value(13, &[0x20, 0x9f]), " \\x9f");
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
