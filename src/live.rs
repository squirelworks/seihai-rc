//! A live RC session on its own thread, for the window.
//!
//! [`Session`] is blocking. The window needs it to keep reading while the
//! person types, so [`LiveSession::start`] logs in on a worker thread, sends
//! every event to the caller as an [`Update`] and takes [`Command`]s from
//! the caller. No async runtime: one link, one thread
//! (`docs/ARCHITECTURE.md`, "Session model").
//!
//! [`RcView`] is what the window draws: the chat log, the player list and
//! the connection state, built only from [`Update`]s. It has no egui in it,
//! so it builds and is tested without the `gui` feature.

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::accounts::{
    account_delete, account_get, account_list, account_set, parse_account, parse_account_list,
    plo as accounts_plo, Account,
};
use crate::files::{delete_message, rename_message, split_path};
use crate::players::{
    admin_message, disconnect_player, parse_props, plo as players_plo, private_admin_message,
    private_message, props_get, props_lines, warp_player,
};
use crate::rc::{
    is_npc_server_player, npc_server_query, parse_npc_endpoint, Event, LoginParams, PlayerEntry,
    PlayerList, Session,
};

/// How long the worker waits for an event before it looks for commands.
const POLL: Duration = Duration::from_millis(50);

/// Something the person asked the session to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// A line in the RC chat.
    Chat(String),
    /// A private message to these player ids.
    Pm { ids: Vec<u16>, text: String },
    /// An admin message to everyone online (63).
    AdminAll(String),
    /// An admin message to one player (64).
    AdminPm { id: u16, text: String },
    /// Disconnect a player (61).
    Kick(u16),
    /// Warp a player (82).
    Warp {
        id: u16,
        level: String,
        x: f32,
        y: f32,
    },
    /// Read a player's props (73); the answer comes as [`Update::Props`].
    Props(u16),
    /// List accounts matching a pattern (72); answer [`Update::Accounts`].
    AccountList(String),
    /// Read one account (77); answer [`Update::Account`].
    AccountGet(String),
    /// Save an account (78; the password is left as it is), then read it
    /// back.
    AccountSet(Account),
    /// Delete an account (71), then list `%` again.
    AccountDelete(String),
    /// List a folder (90); answer [`Update::Dir`].
    FilesList(String),
    /// Fetch a file; answer [`Update::File`].
    FileGet(String),
    /// Send bytes as a file, then list its folder.
    FilePut { path: String, data: Vec<u8> },
    /// Delete a file (97), then list its folder.
    FileDelete(String),
    /// Rename a file in its folder (98), then list the folder.
    FileRename { path: String, new_name: String },
    /// Ask every `(Server)` player where its NPC-server listens (94); the
    /// answers come as [`Update::NpcServer`].
    FindNpcServer,
    /// Log out and end the worker.
    Quit,
}

/// Something the worker tells the window.
#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    /// Logged in to this address.
    Connected(String),
    /// The login failed; the worker has ended.
    LoginFailed(String),
    /// An event from the server, after [`PlayerList::observe`].
    Event(Event),
    /// A command was sent; the text says what, for the log.
    Sent(String),
    /// A command could not be built or sent.
    Error(String),
    /// Props for a player, as the `players props` lines.
    Props { id: u16, lines: Vec<String> },
    /// Account names from a list (70).
    Accounts(Vec<String>),
    /// One account as read (73).
    Account(Account),
    /// The files in a folder.
    Dir { dir: String, names: Vec<String> },
    /// A file's bytes.
    File { path: String, data: Vec<u8> },
    /// An NPC-server address, `HOST:PORT`, from a 79 that parsed.
    NpcServer(String),
    /// The link is closed; the worker has ended.
    Closed(String),
}

/// The worker thread and its two channels.
pub struct LiveSession {
    commands: Sender<Command>,
    updates: Receiver<Update>,
    worker: Option<JoinHandle<()>>,
}

impl LiveSession {
    /// Log in to `addr` on a new thread. The first update is
    /// [`Update::Connected`] or [`Update::LoginFailed`].
    pub fn start(addr: String, params: LoginParams, timeout: Duration) -> Self {
        Self::start_with(addr, params, timeout, None)
    }

    /// As [`Self::start`], with the keepalive interval set (tests use a
    /// short one).
    pub fn start_with(
        addr: String,
        params: LoginParams,
        timeout: Duration,
        keepalive: Option<Duration>,
    ) -> Self {
        let (ctx, crx) = mpsc::channel();
        let (utx, urx) = mpsc::channel();
        let worker = thread::spawn(move || {
            run(addr, params, timeout, keepalive, crx, utx);
        });
        Self {
            commands: ctx,
            updates: urx,
            worker: Some(worker),
        }
    }

    /// Queue a command. False when the worker has already ended.
    pub fn send(&self, c: Command) -> bool {
        self.commands.send(c).is_ok()
    }

    /// Every update that has arrived, without waiting.
    pub fn drain(&self) -> Vec<Update> {
        let mut out = Vec::new();
        while let Ok(u) = self.updates.try_recv() {
            out.push(u);
        }
        out
    }

    /// The next update, waiting at most `timeout`.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<Update> {
        self.updates.recv_timeout(timeout).ok()
    }
}

impl Drop for LiveSession {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Quit);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

/// The bytes for a command, and the log line for it. `None` for
/// [`Command::Quit`].
pub fn encode(c: &Command) -> Option<Result<(Vec<u8>, String), String>> {
    let r = match c {
        Command::Chat(line) => Ok((crate::rc::rc_chat_message(line), String::new())),
        Command::Pm { ids, text } => private_message(ids, text)
            .map(|m| (m, format!("PM to {}: {text}", join_ids(ids))))
            .map_err(|e| e.to_string()),
        Command::AdminAll(text) => admin_message(text)
            .map(|m| (m, format!("admin message to all: {text}")))
            .map_err(|e| e.to_string()),
        Command::AdminPm { id, text } => private_admin_message(*id, text)
            .map(|m| (m, format!("admin message to {id}: {text}")))
            .map_err(|e| e.to_string()),
        Command::Kick(id) => disconnect_player(*id)
            .map(|m| (m, format!("disconnected {id}")))
            .map_err(|e| e.to_string()),
        Command::Warp { id, level, x, y } => warp_player(*id, level, *x, *y)
            .map(|m| (m, format!("warped {id} to {level} {x} {y}")))
            .map_err(|e| e.to_string()),
        Command::Props(id) => props_get(*id)
            .map(|m| (m, String::new()))
            .map_err(|e| e.to_string()),
        Command::AccountList(pattern) => account_list(pattern, "")
            .map(|m| (m, String::new()))
            .map_err(|e| e.to_string()),
        Command::AccountGet(name) => account_get(name)
            .map(|m| (m, String::new()))
            .map_err(|e| e.to_string()),
        Command::AccountSet(a) => account_set(a, "")
            .map(|m| (m, format!("saved account {}", a.account)))
            .map_err(|e| e.to_string()),
        Command::AccountDelete(name) => account_delete(name)
            .map(|m| (m, format!("deleted account {name}")))
            .map_err(|e| e.to_string()),
        // Files and discovery run several steps in the worker.
        Command::FilesList(_)
        | Command::FileGet(_)
        | Command::FilePut { .. }
        | Command::FileDelete(_)
        | Command::FileRename { .. }
        | Command::FindNpcServer => return Some(Err(String::new())),
        Command::Quit => return None,
    };
    Some(r)
}

fn join_ids(ids: &[u16]) -> String {
    ids.iter().map(u16::to_string).collect::<Vec<_>>().join(",")
}

fn run(
    addr: String,
    params: LoginParams,
    timeout: Duration,
    keepalive: Option<Duration>,
    commands: Receiver<Command>,
    updates: Sender<Update>,
) {
    let mut s = match Session::login(&addr, &params, timeout) {
        Ok(s) => s,
        Err(e) => {
            let _ = updates.send(Update::LoginFailed(format!("{addr}: {e}")));
            return;
        }
    };
    if let Some(k) = keepalive {
        s.set_keepalive_every(k);
    }
    if updates.send(Update::Connected(addr)).is_err() {
        s.close();
        return;
    }
    let why = loop {
        // Commands first, so a click goes out within one poll.
        let mut quit = false;
        loop {
            match commands.try_recv() {
                Ok(Command::Quit) => quit = true,
                Ok(c) => execute(&mut s, c, timeout, &updates),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    quit = true;
                    break;
                }
            }
        }
        if quit {
            break "logged out".to_string();
        }
        match s.next_event(Instant::now() + POLL) {
            Ok(Some(ev)) => {
                let update = match &ev {
                    Event::Other(id, p) if *id == players_plo::RC_PLAYERPROPSGET => {
                        match parse_props(p) {
                            Some(r) => Update::Props {
                                id: r.id,
                                lines: props_lines(&r),
                            },
                            None => Update::Error("could not read the props reply".into()),
                        }
                    }
                    Event::Other(id, p) if *id == accounts_plo::RC_ACCOUNTLISTGET => {
                        match parse_account_list(p) {
                            Some(names) => Update::Accounts(names),
                            None => Update::Error("could not read the account list".into()),
                        }
                    }
                    Event::Other(id, p) if *id == accounts_plo::RC_ACCOUNTGET => {
                        match parse_account(p) {
                            Some(a) => Update::Account(a),
                            None => Update::Error("could not read the account".into()),
                        }
                    }
                    Event::NpcServerAddr(a) => {
                        if let Some(ep) = parse_npc_endpoint(&a.raw) {
                            let _ = updates.send(Update::NpcServer(ep.socket_addr()));
                        }
                        Update::Event(ev.clone())
                    }
                    _ => Update::Event(ev.clone()),
                };
                let end = matches!(ev, Event::Disconnect(_));
                if updates.send(update).is_err() {
                    break "window closed".to_string();
                }
                if end {
                    break "the server ended the session".to_string();
                }
            }
            Ok(None) if s.is_closed() => break "the server closed the link".to_string(),
            Ok(None) => {}
            Err(e) => break format!("link error: {e}"),
        }
    };
    s.close();
    let _ = updates.send(Update::Closed(why));
}

/// Run one command on the session.
fn execute(s: &mut Session, c: Command, timeout: Duration, updates: &Sender<Update>) {
    let send = |u: Update| {
        let _ = updates.send(u);
    };
    let list = |s: &mut Session, dir: &str| match s.list_dir(dir, timeout) {
        Ok(names) => send(Update::Dir {
            dir: dir.to_string(),
            names,
        }),
        Err(e) => send(Update::Error(format!("list {dir}: {e}"))),
    };
    match c {
        Command::FilesList(dir) => list(s, &dir),
        Command::FileGet(path) => match s.get_file(&path, timeout) {
            Ok(data) => send(Update::File { path, data }),
            Err(e) => send(Update::Error(format!("get {path}: {e}"))),
        },
        Command::FilePut { path, data } => {
            let n = data.len();
            match s.put_file(&path, &data, timeout) {
                Ok(()) => send(Update::Sent(format!("sent {n} bytes as {path}"))),
                Err(e) => send(Update::Error(format!("put {path}: {e}"))),
            }
            list(s, split_path(&path).0);
        }
        Command::FileDelete(path) => {
            let (dir, name) = split_path(&path);
            file_step(
                s,
                dir,
                delete_message(name),
                &path,
                "deleted",
                timeout,
                updates,
            );
            list(s, dir);
        }
        Command::FileRename { path, new_name } => {
            let (dir, name) = split_path(&path);
            let what = format!("renamed to {new_name}");
            file_step(
                s,
                dir,
                rename_message(name, &new_name),
                &path,
                &what,
                timeout,
                updates,
            );
            list(s, dir);
        }
        Command::FindNpcServer => {
            let ids: Vec<u16> = s
                .players
                .players
                .values()
                .filter(|e| is_npc_server_player(&e.nickname))
                .map(|e| e.id)
                .collect();
            if ids.is_empty() {
                send(Update::Error(
                    "no (Server) player is online; this server has no NPC-server".into(),
                ));
                return;
            }
            let msgs: Vec<Vec<u8>> = ids.into_iter().map(npc_server_query).collect();
            if let Err(e) = s.send_messages(&msgs) {
                send(Update::Error(format!("send failed: {e}")));
            }
        }
        other => {
            let after = match &other {
                Command::AccountSet(a) => account_get(&a.account).ok(),
                Command::AccountDelete(_) => account_list("%", "").ok(),
                _ => None,
            };
            match encode(&other) {
                None => {}
                Some(Err(e)) => send(Update::Error(e)),
                Some(Ok((bytes, note))) => {
                    let mut msgs = vec![bytes];
                    msgs.extend(after);
                    match s.send_messages(&msgs) {
                        Ok(()) if note.is_empty() => {}
                        Ok(()) => send(Update::Sent(note)),
                        Err(e) => send(Update::Error(format!("send failed: {e}"))),
                    }
                }
            }
        }
    }
}

/// List `dir` (the fork names a file by the folder of the last list), send
/// `msg`, and report the server's id 67 lines.
fn file_step(
    s: &mut Session,
    dir: &str,
    msg: Result<Vec<u8>, crate::files::FileError>,
    path: &str,
    done: &str,
    timeout: Duration,
    updates: &Sender<Update>,
) {
    let send = |u: Update| {
        let _ = updates.send(u);
    };
    let msg = match msg {
        Ok(m) => m,
        Err(e) => return send(Update::Error(format!("{path}: {e}"))),
    };
    if let Err(e) = s.list_dir(dir, timeout) {
        return send(Update::Error(format!("list {dir}: {e}")));
    }
    match s.file_action(msg, Duration::from_millis(500), Instant::now() + timeout) {
        Ok(lines) => {
            for line in lines {
                send(Update::Sent(format!("[server] {line}")));
            }
            send(Update::Sent(format!("{path} {done}")));
        }
        Err(e) => send(Update::Error(format!("{path}: {e}"))),
    }
}

/// Connection state, for the window's status line.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    LoggedOut,
    Connecting,
    Online(String),
    Failed(String),
    Closed(String),
}

/// What the window shows, built from [`Update`]s only.
#[derive(Debug, Default)]
pub struct RcView {
    pub status: Status,
    pub players: PlayerList,
    /// The log, oldest first, at most [`Self::LOG_MAX`] lines.
    pub log: Vec<String>,
    /// The last props read, by player id.
    pub props: Option<(u16, Vec<String>)>,
    /// Log events the command line prints only with `-v`.
    pub verbose: bool,
    /// The last account list.
    pub accounts: Vec<String>,
    /// The last account read.
    pub account: Option<Account>,
    /// The last folder listed and its files.
    pub dir: Option<(String, Vec<String>)>,
    /// The last file fetched, for the window to save.
    pub file: Option<(String, Vec<u8>)>,
    /// NPC-server addresses found on this server.
    pub npc_servers: Vec<String>,
}

impl RcView {
    /// Older lines are dropped past this many.
    pub const LOG_MAX: usize = 5000;

    pub fn apply(&mut self, u: Update) {
        match u {
            Update::Connected(addr) => {
                self.push(format!("[connected] {addr}"));
                self.status = Status::Online(addr);
            }
            Update::LoginFailed(why) => {
                self.push(format!("[login failed] {why}"));
                self.status = Status::Failed(why);
            }
            Update::Event(ev) => {
                self.players.apply(&ev);
                if let Some(line) = event_line(&ev, self.verbose) {
                    self.push(line);
                }
            }
            Update::Sent(note) => self.push(format!("[sent] {note}")),
            Update::Error(e) => self.push(format!("[error] {e}")),
            Update::Props { id, lines } => self.props = Some((id, lines)),
            Update::Accounts(names) => {
                self.push(format!("[accounts] {} listed", names.len()));
                self.accounts = names;
            }
            Update::Account(a) => self.account = Some(a),
            Update::Dir { dir, names } => self.dir = Some((dir, names)),
            Update::File { path, data } => {
                self.push(format!("[file] {path}: {} bytes", data.len()));
                self.file = Some((path, data));
            }
            Update::NpcServer(addr) => {
                // The event line already shows the address as sent.
                if !self.npc_servers.contains(&addr) {
                    self.npc_servers.push(addr);
                }
            }
            Update::Closed(why) => {
                self.push(format!("[closed] {why}"));
                self.status = Status::Closed(why);
                self.players = PlayerList::default();
            }
        }
    }

    fn push(&mut self, line: String) {
        self.log.push(line);
        if self.log.len() > Self::LOG_MAX {
            let extra = self.log.len() - Self::LOG_MAX;
            self.log.drain(..extra);
        }
    }

    /// Players first, then RCs, each by id.
    pub fn sorted_players(&self) -> Vec<&PlayerEntry> {
        let mut v: Vec<&PlayerEntry> = self.players.players.values().collect();
        v.sort_by_key(|e| (e.is_rc(), e.id));
        v
    }
}

/// One `login` line for an arrival or a props refresh.
pub fn player_line(tag: &str, e: &PlayerEntry) -> String {
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

/// The line `seihai-rc login` prints for an event, if any. With `verbose`
/// the events it prints to stderr come back too.
pub fn event_line(ev: &Event, verbose: bool) -> Option<String> {
    Some(match ev {
        Event::Signature => return None,
        Event::Disconnect(why) => format!("[disconnected] {why}"),
        Event::RcChat(line) => format!("[rc] {line}"),
        Event::AdminMessage(m) => format!("[admin] {}", m.replace('\u{a7}', ": ")),
        Event::PrivateMessage { from, text } => format!("[pm from {from}] {text}"),
        Event::AddPlayer(e) => player_line("online", e),
        Event::UpdatePlayer(e) => player_line("update", e),
        Event::DelPlayer(id) => format!("[offline] {id}"),
        Event::OtherProps(id, _) if verbose => format!("[id 8] props for unlisted {id}"),
        Event::OtherProps(..) => return None,
        Event::NpcServerAddr(addr) => format!("[npc-server] {} {}", addr.player_id, addr.raw),
        Event::PlayerFlags(raw) => format!("[player-flags] {}", hex_bytes(raw)),
        Event::Other(id, p) if verbose => format!("[id {id}] {} bytes", p.len()),
        Event::Other(..) => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u16, account: &str, level: &str) -> PlayerEntry {
        PlayerEntry {
            id,
            account: account.into(),
            nickname: account.into(),
            level: level.into(),
            ..Default::default()
        }
    }

    #[test]
    fn view_tracks_players_and_log() {
        let mut v = RcView::default();
        v.apply(Update::Connected("127.0.0.1:1".into()));
        assert_eq!(v.status, Status::Online("127.0.0.1:1".into()));
        v.apply(Update::Event(Event::AddPlayer(entry(2, "bob", "cave.nw"))));
        v.apply(Update::Event(Event::AddPlayer(entry(1, "labrc2", " "))));
        v.apply(Update::Event(Event::AddPlayer(entry(3, "amy", "start.nw"))));
        let ids: Vec<u16> = v.sorted_players().iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![2, 3, 1], "players by id, then RCs");
        v.apply(Update::Event(Event::DelPlayer(2)));
        v.apply(Update::Event(Event::RcChat("hello".into())));
        v.apply(Update::Event(Event::Signature));
        assert_eq!(
            v.log,
            vec![
                "[connected] 127.0.0.1:1",
                "[online] 2 bob (cave.nw)",
                "[online] 1 labrc2 (RC)",
                "[online] 3 amy (start.nw)",
                "[offline] 2",
                "[rc] hello",
            ]
        );
        v.apply(Update::Closed("logged out".into()));
        assert!(v.players.players.is_empty());
        assert_eq!(v.status, Status::Closed("logged out".into()));
    }

    #[test]
    fn view_log_is_bounded() {
        let mut v = RcView::default();
        for i in 0..RcView::LOG_MAX + 10 {
            v.apply(Update::Sent(i.to_string()));
        }
        assert_eq!(v.log.len(), RcView::LOG_MAX);
        assert_eq!(v.log[0], "[sent] 10");
    }

    #[test]
    fn verbose_shows_other_ids() {
        let mut v = RcView::default();
        v.apply(Update::Event(Event::Other(124, vec![1, 2])));
        assert!(v.log.is_empty());
        v.verbose = true;
        v.apply(Update::Event(Event::Other(124, vec![1, 2])));
        assert_eq!(v.log, vec!["[id 124] 2 bytes"]);
    }

    #[test]
    fn commands_encode_as_the_cli_does() {
        let (b, note) = encode(&Command::Kick(5)).unwrap().unwrap();
        assert_eq!(b, disconnect_player(5).unwrap());
        assert_eq!(note, "disconnected 5");
        let (b, _) = encode(&Command::Warp {
            id: 2,
            level: "cave.nw".into(),
            x: 30.5,
            y: 12.0,
        })
        .unwrap()
        .unwrap();
        assert_eq!(b, warp_player(2, "cave.nw", 30.5, 12.0).unwrap());
        assert!(encode(&Command::Warp {
            id: 2,
            level: "cave.nw".into(),
            x: 30.3,
            y: 1.0
        })
        .unwrap()
        .is_err());
        assert!(encode(&Command::Pm {
            ids: vec![],
            text: "x".into()
        })
        .unwrap()
        .is_err());
        assert!(encode(&Command::Quit).is_none());
    }
}

/// The window's login fields, checked into a [`LoginParams`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginForm {
    /// `HOST:PORT` of the game server.
    pub server: String,
    pub account: String,
    pub password: String,
    /// Empty means `*account`, as the command line sends.
    pub nickname: String,
    pub classic: bool,
}

impl Default for LoginForm {
    fn default() -> Self {
        Self {
            server: "127.0.0.1:14802".into(),
            account: String::new(),
            password: String::new(),
            nickname: String::new(),
            classic: false,
        }
    }
}

impl LoginForm {
    /// The address and login, or what is missing. `key` is the gen-5 key
    /// (any value from 0 to 127).
    pub fn params(&self, key: u8) -> Result<(String, LoginParams), String> {
        let server = self.server.trim();
        match server.rsplit_once(':') {
            Some((host, port)) if !host.is_empty() && port.parse::<u16>().is_ok() => {}
            _ => return Err("the server is HOST:PORT".into()),
        }
        let account = self.account.trim();
        if account.is_empty() {
            return Err("the account is empty".into());
        }
        if self.password.is_empty() {
            return Err("the password is empty".into());
        }
        let kind = if self.classic {
            crate::rc::RcKind::Classic
        } else {
            crate::rc::RcKind::Gen5
        };
        let nick = self.nickname.trim();
        Ok((
            server.to_string(),
            LoginParams {
                kind,
                token: kind.default_token().into(),
                account: account.into(),
                password: self.password.clone(),
                key: key & 0x7F,
                nickname: Some(if nick.is_empty() {
                    format!("*{account}")
                } else {
                    nick.into()
                }),
                identity: None,
            },
        ))
    }
}

/// A warp from the window's text fields.
pub fn warp_command(id: u16, level: &str, x: &str, y: &str) -> Result<Command, String> {
    let level = level.trim();
    if level.is_empty() {
        return Err("the level is empty".into());
    }
    let num = |axis: &str, v: &str| {
        v.trim()
            .parse::<f32>()
            .map_err(|_| format!("{axis} is not a number: {v:?}"))
    };
    Ok(Command::Warp {
        id,
        level: level.into(),
        x: num("x", x)?,
        y: num("y", y)?,
    })
}

#[cfg(test)]
mod form_tests {
    use super::*;
    use crate::rc::RcKind;

    #[test]
    fn login_form_checks_fields() {
        let mut f = LoginForm {
            account: "labrc".into(),
            password: "pw".into(),
            ..Default::default()
        };
        let (addr, p) = f.params(0xFF).unwrap();
        assert_eq!(addr, "127.0.0.1:14802");
        assert_eq!(p.kind, RcKind::Gen5);
        assert_eq!(p.key, 0x7F);
        assert_eq!(p.nickname.as_deref(), Some("*labrc"));
        assert_eq!(p.token, RcKind::Gen5.default_token());
        f.classic = true;
        f.nickname = " Lab RC ".into();
        let (_, p) = f.params(1).unwrap();
        assert_eq!(p.kind, RcKind::Classic);
        assert_eq!(p.nickname.as_deref(), Some("Lab RC"));
        f.server = "127.0.0.1".into();
        assert!(f.params(1).is_err());
        f.server = "127.0.0.1:99999".into();
        assert!(f.params(1).is_err());
        f.server = "127.0.0.1:1".into();
        f.password.clear();
        assert!(f.params(1).is_err());
    }

    #[test]
    fn warp_fields_parse() {
        assert_eq!(
            warp_command(2, " cave.nw ", "30.5", " 12").unwrap(),
            Command::Warp {
                id: 2,
                level: "cave.nw".into(),
                x: 30.5,
                y: 12.0
            }
        );
        assert!(warp_command(2, "", "1", "1").is_err());
        assert!(warp_command(2, "a.nw", "x", "1").is_err());
    }
}
