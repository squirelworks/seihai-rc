//! seihai-rc against the real seihai-listserver (a dev-dependency at a
//! tag), started on free loopback ports, instead of the fake listserver in
//! `tests/fake_servers.rs`. seihai-listserver issue #19, item 2.
//!
//! Two gservers register: `Quiet` with HQ level 0 (hidden: only the RC
//! list shows it, tagged `U `) and `TestWorld` with HQ level gold (`P `).

use std::io::Write;
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use seihai_core::protocol::zlib_compress;
use seihai_listserver::{Config, Running};
use seihai_rc::listserver::{fetch_rc_list, ListError};

const TIMEOUT: Duration = Duration::from_secs(10);

fn gchar(n: usize) -> u8 {
    (n + 32) as u8
}

fn gstr(s: &[u8]) -> Vec<u8> {
    [&[gchar(s.len())][..], s].concat()
}

/// A scratch folder for the accounts file and the listserver's key.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("seihai-rc-ls-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Scratch(p)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The listserver on free loopback ports, with one account, `labrc`.
fn start(dir: &Scratch) -> Running {
    let accounts = dir.0.join("accounts.txt");
    seihai_listserver::accounts::add_account(&accounts, "labrc", b"labpw").unwrap();
    seihai_listserver::start(Config {
        gserver_port: 0,
        client_ports: vec![0],
        accounts,
        secure_login_key: dir.0.join("seihai-login.key"),
        ..Config::default()
    })
    .unwrap()
}

/// Register a gserver over the gserver link (`REGISTERV3`, the HQ level,
/// `NEWSERVER`). Keep the stream: closing it drops the entry.
fn register(run: &Running, name: &[u8], hq_level: usize) -> TcpStream {
    let mut g = TcpStream::connect(run.gserver_addr).unwrap();
    g.write_all(&[&[gchar(30)][..], b"lab-test\n"].concat())
        .unwrap();
    let fields: [&[u8]; 8] = [
        name,
        b"A test server",
        b"English",
        b"3.0.9",
        b"http://x/",
        b"AUTO",
        b"14802",
        b"",
    ];
    let newserver = [
        vec![gchar(22)],
        fields.iter().flat_map(|f| gstr(f)).collect(),
    ]
    .concat();
    let msgs = [vec![gchar(24), gchar(hq_level)], newserver];
    let body: Vec<u8> = msgs.iter().flat_map(|m| [&m[..], b"\n"].concat()).collect();
    let z = zlib_compress(&body).unwrap();
    g.write_all(&[&(z.len() as u16).to_be_bytes()[..], &z].concat())
        .unwrap();
    g
}

fn wait_listed(run: &Running, n: usize) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while run.shared.servers.listed() != n {
        assert!(Instant::now() < deadline, "listed count never reached {n}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn rc_list_from_the_real_listserver_shows_the_hidden_server() {
    let dir = Scratch::new("list");
    let run = start(&dir);
    let _quiet = register(&run, b"Quiet", 0);
    let _gold = register(&run, b"TestWorld", 3);
    wait_listed(&run, 2);
    let addr = run.client_addrs[0].to_string();

    let reply = fetch_rc_list(&addr, "GSERV025", "labrc", "labpw", 0x35, TIMEOUT).unwrap();
    let got: Vec<(&str, &str)> = reply
        .servers
        .iter()
        .map(|s| (s.kind.as_str(), s.name.as_str()))
        .collect();
    assert_eq!(got, [("U", "Quiet"), ("P", "TestWorld")]);
    assert_eq!(reply.servers[1].port, 14802);
    assert!(
        reply
            .status
            .as_deref()
            .unwrap_or_default()
            .contains("There are 2 server(s) online."),
        "{:?}",
        reply.status
    );

    // The command line prints the same two servers.
    let out = Command::new(env!("CARGO_BIN_EXE_seihai-rc"))
        .args(["servers", "--listserver", &addr, "--account", "labrc"])
        .env("SEIHAI_RC_PASSWORD", "labpw")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Quiet") && stdout.contains("TestWorld"),
        "{stdout}"
    );
}

#[test]
fn wrong_password_is_refused_by_the_real_listserver() {
    let dir = Scratch::new("refused");
    let run = start(&dir);
    let addr = run.client_addrs[0].to_string();
    match fetch_rc_list(&addr, "GSERV025", "labrc", "nope", 0x35, TIMEOUT) {
        Err(ListError::Refused(why)) => assert!(!why.is_empty()),
        other => panic!("expected a refusal, got {other:?}"),
    }
}
