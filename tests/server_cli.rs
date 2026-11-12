//! `seihai-rc server options|folders|flags` against a fake gserver that
//! keeps the three values. The layouts are **check** (see `src/server.rs`);
//! the fake reads and writes them that way.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Output};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use seihai_core::protocol::{read_gshort, split_messages, zlib_decompress};
use seihai_rc::link::Codec;
use seihai_rc::wire::{gchar, message, push_gshort, push_gstring, read_gstring};

const TIMEOUT: Duration = Duration::from_secs(5);

fn read_frame(s: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut len = [0u8; 2];
    s.read_exact(&mut len)?;
    let mut f = vec![0u8; usize::from(u16::from_be_bytes(len))];
    s.read_exact(&mut f)?;
    Ok(f)
}

fn send(s: &mut TcpStream, codec: &mut Codec, msgs: &[Vec<u8>]) {
    let mut body = Vec::new();
    for m in msgs {
        body.extend_from_slice(m);
        body.push(b'\n');
    }
    s.write_all(&codec.encode(&body).unwrap()).unwrap();
}

struct State {
    options: Vec<u8>,
    folders: Vec<u8>,
    flags: Vec<Vec<u8>>,
    /// Ids the RC sent, in order (nickname left out).
    seen: Vec<u8>,
}

/// `drop_last_flag`: store all but the last flag of a set, as a server
/// that refused one would.
fn fake(drop_last_flag: bool) -> (String, JoinHandle<State>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap().to_string();
    let h = thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        s.set_read_timeout(Some(TIMEOUT)).unwrap();
        let login = zlib_decompress(&read_frame(&mut s).unwrap()).unwrap();
        let mut codec = Codec::gen5(login[1] - 0x20);
        send(&mut s, &mut codec, &[message(25, &[gchar(73)])]);
        let mut st = State {
            options: b"\"name=Lab\",\"staff=labrc\"".to_vec(),
            folders: b"\"file rw levels/*\"".to_vec(),
            flags: vec![b"lab=1".to_vec()],
            seen: Vec::new(),
        };
        while let Ok(frame) = read_frame(&mut s) {
            for m in split_messages(&codec.decode(&frame).unwrap()) {
                if m.id == 2 {
                    continue;
                }
                st.seen.push(m.id);
                let out = match m.id {
                    51 => message(76, &st.options),
                    53 => message(77, &st.folders),
                    68 => {
                        let mut p = Vec::new();
                        push_gshort(&mut p, st.flags.len() as u16);
                        for f in &st.flags {
                            push_gstring(&mut p, f);
                        }
                        message(61, &p)
                    }
                    52 => {
                        st.options = m.payload.clone();
                        continue;
                    }
                    54 => {
                        st.folders = m.payload.clone();
                        continue;
                    }
                    69 => {
                        let n = read_gshort(&m.payload).unwrap();
                        let mut i = 2;
                        st.flags = (0..n)
                            .map(|_| read_gstring(&m.payload, &mut i).unwrap())
                            .collect();
                        if drop_last_flag {
                            st.flags.pop();
                        }
                        continue;
                    }
                    _ => continue,
                };
                send(&mut s, &mut codec, &[out]);
            }
        }
        st
    });
    (addr, h)
}

fn cli(addr: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_seihai-rc"))
        .arg("server")
        .args(args)
        .args([
            "--server",
            addr,
            "--account",
            "labrc",
            "--settle",
            "200",
            "--timeout",
            "5",
        ])
        .env("SEIHAI_RC_PASSWORD", "labpw")
        .output()
        .unwrap()
}

fn temp(name: &str, body: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("seihai-rc-{name}-{}", std::process::id()));
    std::fs::write(&p, body).unwrap();
    p
}

#[test]
fn options_get_prints_one_per_line() {
    let (addr, h) = fake(false);
    let o = cli(&addr, &["options"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(
        String::from_utf8_lossy(&o.stdout),
        "name=Lab\nstaff=labrc\n"
    );
    assert_eq!(h.join().unwrap().seen, [51]);
}

#[test]
fn folders_set_sends_54_and_reads_back() {
    let f = temp("folders", "file rw levels/*\nfile r images/*\n");
    let (addr, h) = fake(false);
    let o = cli(&addr, &["folders", "--set-file", f.to_str().unwrap()]);
    std::fs::remove_file(&f).unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(String::from_utf8_lossy(&o.stderr).contains("1 line(s) before, 2 after"));
    let st = h.join().unwrap();
    assert_eq!(st.seen, [53, 54, 53]);
    assert_eq!(st.folders, b"\"file rw levels/*\",\"file r images/*\"");
}

#[test]
fn flags_set_sends_69_and_reads_back() {
    let f = temp("flags", "lab=1\nevent=on\n");
    let (addr, h) = fake(false);
    let o = cli(&addr, &["flags", "--set-file", f.to_str().unwrap()]);
    std::fs::remove_file(&f).unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(String::from_utf8_lossy(&o.stdout), "lab=1\nevent=on\n");
    let st = h.join().unwrap();
    assert_eq!(st.seen, [68, 69, 68]);
    assert_eq!(st.flags, [b"lab=1".to_vec(), b"event=on".to_vec()]);
}

#[test]
fn a_flag_the_server_did_not_keep_fails_the_set() {
    let f = temp("flags-lossy", "lab=1\nevent=on\n");
    let (addr, h) = fake(true);
    let o = cli(&addr, &["flags", "--set-file", f.to_str().unwrap()]);
    std::fs::remove_file(&f).unwrap();
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("other flags than were sent"));
    h.join().unwrap();
}
