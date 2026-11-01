//! `seihai-rc`: the command line.

use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use seihai_rc::listserver::{fetch_rc_list, pick};
use seihai_rc::nc::{NcEvent, NcSession};
use seihai_rc::rc::{Event, LoginParams, PlayerEntry, RcKind, Session};

const USAGE: &str = "\
seihai-rc: Graal Remote Control client (stage 1, plus NPC-Control login)

USAGE:
  seihai-rc servers --listserver HOST:PORT --account NAME [options]
  seihai-rc login   (--server HOST:PORT | --listserver HOST:PORT --pick NAME|N)
                    --account NAME [--for SECS] [--say TEXT] [options]
  seihai-rc players list (--server ... | --listserver ... --pick ...)
                    --account NAME [--tsv] [--settle MS] [options]
  seihai-rc nc login --server HOST:PORT --account NAME [--for SECS] [options]

  servers       Log in to the listserver's RC list and print every server.
  login         Log in to a gserver and print server messages and the RC
                chat for --for seconds (default 5; 0 = until the server
                closes the link). --say sends one RC chat line first.
  players list  Log in and print the players and RCs online.
  nc login      Log in to an NPC-server (type 3, plain zlib) and print
                message ids for --for seconds. The address is the one you
                pass; finding it from the game server is a later step.

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
  --settle MS          players list: wait until no message came for this
                       long before printing (default 1000).
  --tsv                players list: tab-separated, with a header.
  -v, --verbose        login: also show the ids of messages not read yet.
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
    tsv: bool,
    verbose: bool,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Opts, String> {
    let mut o = Opts::default();
    let mut it = args.peekable();
    while let Some(a) = it.next() {
        let mut val = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        let num = |name: &str, v: String| {
            v.parse::<u64>()
                .map_err(|_| format!("{name}: not a number: {v}"))
        };
        match a.as_str() {
            "--server" => o.server = Some(val(&a)?),
            "--listserver" => o.listserver = Some(val(&a)?),
            "--pick" => o.pick = Some(val(&a)?),
            "--account" => o.account = Some(val(&a)?),
            "--password-env" => o.password_env = Some(val(&a)?),
            "--rc" => o.rc = Some(val(&a)?),
            "--token" => o.token = Some(val(&a)?),
            "--nick" => o.nick = Some(val(&a)?),
            "--say" => o.say = Some(val(&a)?),
            "--timeout" => o.timeout = Some(num(&a, val(&a)?)?),
            "--settle" => o.settle = Some(num(&a, val(&a)?)?),
            "--for" => o.for_secs = Some(num(&a, val(&a)?)?),
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
            let reply = fetch_rc_list(ls, &c.token, &c.account, &c.password, c.key, c.timeout)
                .map_err(|e| format!("{ls}: {e}"))?;
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
        Event::Other(id, p) if verbose => eprintln!("[id {id}] {} bytes", p.len()),
        Event::Other(..) => {}
    }
}

fn cmd_servers(o: &Opts) -> Result<(), String> {
    let c = common(o)?;
    let ls = o.listserver.as_deref().ok_or("--listserver is required")?;
    let reply = fetch_rc_list(ls, &c.token, &c.account, &c.password, c.key, c.timeout)
        .map_err(|e| format!("{ls}: {e}"))?;
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
        ["nc", "login"] => cmd_nc_login(&o),
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
}
