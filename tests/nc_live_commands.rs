//! Run the shipped NC commands against the real loopback gserver and its
//! external NPC-server session. These are not protocol fakes: the CLI,
//! gserver, NPC Control link, and NPC-server all run as compiled Rust code.

use seihai_core::scripting::engine::Era;
use seihai_gserver::accounts::{Account, Store};
use seihai_gserver::npclink::serve_link;
use seihai_gserver::server::serve;
use seihai_npcserver::scripts::Scripts;
use seihai_npcserver::session::{self, Timing};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PASSWORD: &str = "labpw";
const TOKEN: &str = "lab-token-0123456789";
const TIMEOUT: Duration = Duration::from_secs(10);

struct Scratch(PathBuf);

impl Scratch {
    fn world() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("seihai-rc-nc-live-{}-{nonce}", std::process::id()));
        for child in ["config", "levels", "npcs"] {
            std::fs::create_dir_all(dir.join(child)).unwrap();
        }
        std::fs::write(dir.join("levels/start.nw"), "GLEVNW01\n").unwrap();
        std::fs::write(dir.join("levels/house.nw"), "GLEVNW01\n").unwrap();
        std::fs::write(
            dir.join("config/server.txt"),
            format!(
                "name = RC NC loopback\nheartbeat = 1\nnpcserver = external\nnpcserver_token = {TOKEN}\n"
            ),
        )
        .unwrap();
        std::fs::write(dir.join("config/allowedversions.txt"), "*\n").unwrap();
        std::fs::write(
            dir.join("npcs/bell.txt"),
            "LEVEL start.nw\nX 5\nY 6\nIMAGE bell.png\nSCRIPT\nif (created) { message ding; }\nSCRIPTEND\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("npcs/npcControl.txt"),
            "GRNPC001\nNAME Control\nTYPE CONTROL\nSTARTLEVEL \nNPCSCRIPT\nif (playerlogin) { message x; }\nNPCSCRIPTEND\n",
        )
        .unwrap();
        let accounts = Store::new(dir.join("accounts"));
        let mut boss = Account::new("boss", "start.nw", 30.0, 30.0);
        boss.set_password(PASSWORD.as_bytes());
        boss.staff = true;
        accounts.save(&boss).unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct LiveWorld {
    game: String,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    server_thread: Option<thread::JoinHandle<()>>,
    npc_thread: Option<thread::JoinHandle<()>>,
}

impl LiveWorld {
    fn start(world: &Path) -> Self {
        let (server, warnings) = seihai_gserver::load_world(world).unwrap();
        assert!(warnings.is_empty(), "gserver world warnings: {warnings:?}");
        let server = Arc::new(server);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
        let server_thread = thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async move {
                let game = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let link = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let game_addr = game.local_addr().unwrap().to_string();
                let link_addr = link.local_addr().unwrap().to_string();
                let game_task = tokio::spawn(serve(server.clone(), game));
                let link_task = tokio::spawn(serve_link(server.npc.clone(), link));
                ready_tx.send((game_addr, link_addr)).unwrap();
                let _ = stop_rx.await;
                game_task.abort();
                link_task.abort();
                let _ = game_task.await;
                let _ = link_task.await;
            });
        });
        let (game, npc_link) = ready_rx.recv().unwrap();

        let npc_world = world.to_path_buf();
        let link_addr = npc_link.clone();
        let npc_thread = thread::spawn(move || {
            let mut scripts = Scripts::with_world(Era::V217, &npc_world);
            let timing = Timing {
                tick: Duration::from_millis(20),
                ..Timing::default()
            };
            let _ = session::run(
                &link_addr,
                TOKEN.as_bytes(),
                timing,
                Some(&mut scripts),
                &mut |_| {},
            );
        });

        Self {
            game,
            stop: Some(stop_tx),
            server_thread: Some(server_thread),
            npc_thread: Some(npc_thread),
        }
    }

    fn run_nc(&self, args: &[&str]) -> Output {
        self.run_nc_with_settle(args, 150)
    }

    fn run_nc_with_settle(&self, args: &[&str], settle_ms: u64) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_seihai-rc"));
        let settle_ms = settle_ms.to_string();
        command.args(args).args([
            "--server",
            &self.game,
            "--account",
            "boss",
            "--timeout",
            "10",
            "--settle",
            &settle_ms,
        ]);
        command
            .env("SEIHAI_RC_PASSWORD", PASSWORD)
            .output()
            .unwrap()
    }
}

impl Drop for LiveWorld {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.npc_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.server_thread.take() {
            let _ = thread.join();
        }
    }
}

fn successful(output: Output, command: &str) -> String {
    assert!(
        output.status.success(),
        "{command} failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn wait_for_file(path: &Path, expected: &str) -> String {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let contents = std::fs::read_to_string(path).unwrap();
        if contents.starts_with(expected) {
            return contents;
        }
        assert!(
            Instant::now() < deadline,
            "expected {expected:?}, got {contents:?}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn shipped_nc_add_warp_reset_and_cleanup_run_against_live_servers() {
    let world = Scratch::world();
    let live = LiveWorld::start(&world.0);

    let deadline = Instant::now() + TIMEOUT;
    loop {
        let initial = successful(
            live.run_nc_with_settle(&["nc", "npc", "list", "--tsv"], 1000),
            "nc npc list before mutation",
        );
        if initial.lines().any(|line| line.starts_with("1\tbell\t")) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the real NPC-server did not publish the seeded NPC: {initial}"
        );
    }

    let initial = successful(
        live.run_nc(&["nc", "npc", "get", "1", "--verbose"]),
        "nc npc get 1 before mutation",
    );
    assert!(
        initial.contains("name=bell") && initial.contains("level=start.nw"),
        "live NC did not read the seeded NPC: {initial}"
    );

    assert_eq!(
        successful(live.run_nc(&["nc", "npc", "reset", "1"]), "nc npc reset 1"),
        "reset 1\n"
    );

    assert_eq!(
        successful(
            live.run_nc(&["nc", "npc", "warp", "1", "house.nw", "12.5", "7"]),
            "nc npc warp 1"
        ),
        "sent warp for 1\n"
    );
    let warped = wait_for_file(
        &world.0.join("npcs/bell.txt"),
        "LEVEL house.nw\nX 12.5\nY 7\n",
    );
    assert!(warped.contains("SCRIPTEND"), "warp lost NPC data: {warped}");
    let readback = successful(live.run_nc(&["nc", "npc", "get", "1"]), "nc npc get 1");
    assert!(
        readback.contains("level=house.nw")
            && readback.contains("x=12.5")
            && readback.contains("y=7"),
        "live NC readback did not show the warp: {readback}"
    );

    let add_output = live.run_nc(&[
        "nc", "npc", "add", "Guard", "1001", "OBJECT", "start.nw", "20", "25",
    ]);
    let add_stderr = String::from_utf8_lossy(&add_output.stderr).into_owned();
    let added = successful(add_output, "nc npc add Guard 1001");
    let added_id = added
        .strip_prefix("added ")
        .and_then(|s| s.trim().parse::<u32>().ok())
        .expect("add should print the id assigned by the server");
    assert_eq!(added_id, 3, "gserver allocates IDs after the seeded NPCs");
    assert!(
        add_stderr.contains("server assigned NPC id 3 (requested 1001)"),
        "CLI should explain the server-assigned ID: {add_stderr}"
    );
    assert!(world.0.join("npcs/npcGuard.txt").is_file());
    let readback = successful(
        live.run_nc(&["nc", "npc", "get", &added_id.to_string()]),
        "nc npc get the assigned id after add",
    );
    assert!(
        readback.contains("name=Guard") && readback.contains("level=start.nw"),
        "live NC readback did not show the added NPC: {readback}"
    );

    assert_eq!(
        successful(
            live.run_nc(&["nc", "npc", "delete", &added_id.to_string()]),
            "nc npc delete the assigned id"
        ),
        format!("deleted {added_id}\n")
    );
    assert!(world.0.join("npcs/npcGuard.txt.deleted").is_file());
    let listing = successful(
        live.run_nc(&["nc", "npc", "list", "--tsv"]),
        "nc npc list after delete",
    );
    let deleted_prefix = format!("{added_id}\t");
    assert!(
        !listing
            .lines()
            .any(|line| line.starts_with(&deleted_prefix)),
        "deleted NPC remained in live readback: {listing}"
    );
}
