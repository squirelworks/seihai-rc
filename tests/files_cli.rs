//! `seihai-rc files ...` against a fake gserver that keeps a small world.
//!
//! The move, delete, rename and folder-delete layouts are **check**: the
//! fake reads them the way `src/files.rs` documents them. The fake names a
//! file by the folder from the last list, as the fork does.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Output};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use seihai_core::protocol::{read_gint3, split_messages, zlib_decompress};
use seihai_rc::link::Codec;
use seihai_rc::wire::{gchar, message, push_gint5, push_gstring, read_gstring, text};

const TIMEOUT: Duration = Duration::from_secs(5);

type World = BTreeMap<String, BTreeMap<String, Vec<u8>>>;

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

fn dir(folder: &str, files: &BTreeMap<String, Vec<u8>>) -> Vec<Vec<u8>> {
    let mut p = Vec::new();
    push_gstring(&mut p, folder.as_bytes());
    for (name, data) in files {
        let mut rec = Vec::new();
        push_gstring(&mut rec, name.as_bytes());
        push_gstring(&mut rec, b"rw");
        push_gint5(&mut rec, data.len() as u32);
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

/// What the fake saw, as `id payload-text` lines, and the world at the end.
fn fake(mut world: World) -> (String, JoinHandle<(Vec<String>, World)>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap().to_string();
    let h = thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        s.set_read_timeout(Some(TIMEOUT)).unwrap();
        let login = zlib_decompress(&read_frame(&mut s).unwrap()).unwrap();
        let mut codec = Codec::gen5(login[1] - 0x20);
        send(&mut s, &mut codec, &[message(25, &[gchar(73)])]);
        let mut seen = Vec::new();
        let mut cwd = String::new();
        while let Ok(frame) = read_frame(&mut s) {
            let body = codec.decode(&frame).unwrap();
            if body[0].wrapping_sub(0x20) == 50 {
                // Put: raw block, id 93, GSTRING name, the bytes.
                // The count includes the block's last line feed; the
                // fork drops it, so a short count would cost the file's
                // own last byte.
                let n = read_gint3(&body[1..4]).unwrap();
                assert_eq!(body.len(), 5 + n);
                assert_eq!(body[4 + n], b'\n');
                let inner = &body[5..4 + n];
                let mut i = 1;
                let name = text(&read_gstring(inner, &mut i).unwrap());
                seen.push(format!("93 {name}"));
                world
                    .get_mut(&cwd)
                    .unwrap()
                    .insert(name.clone(), inner[i..].to_vec());
                let ack = format!("Uploaded file {name}");
                send(&mut s, &mut codec, &[message(67, ack.as_bytes())]);
                continue;
            }
            for m in split_messages(&body) {
                let p = &m.payload;
                let out = match m.id {
                    2 => continue,
                    90 => {
                        let f = text(p);
                        seen.push(format!("90 {f}"));
                        match world.get(&f) {
                            Some(files) => {
                                cwd = f.clone();
                                dir(&f, files)
                            }
                            None => vec![message(67, b"no such folder")],
                        }
                    }
                    92 => {
                        let name = text(p);
                        seen.push(format!("92 {name}"));
                        let data = world[&cwd][&name].clone();
                        let mut packet = vec![gchar(102)];
                        push_gint5(&mut packet, 0);
                        push_gstring(&mut packet, name.as_bytes());
                        packet.extend_from_slice(&data);
                        let n = packet.len() + 1;
                        let mut raw = vec![gchar(100)];
                        raw.extend_from_slice(&[
                            gchar(((n >> 14) & 0x7F) as u8),
                            gchar(((n >> 7) & 0x7F) as u8),
                            gchar((n & 0x7F) as u8),
                            b'\n',
                        ]);
                        raw.extend_from_slice(&packet);
                        s.write_all(&codec.encode(&[raw, b"\n".to_vec()].concat()).unwrap())
                            .unwrap();
                        continue;
                    }
                    96 => {
                        let mut i = 0;
                        // The fork's order: GSTRING folder, then the name.
                        let to = text(&read_gstring(p, &mut i).unwrap());
                        let name = text(&p[i..]);
                        seen.push(format!("96 {name} {to}"));
                        let data = world.get_mut(&cwd).unwrap().remove(&name).unwrap();
                        world.get_mut(&to).unwrap().insert(name.clone(), data);
                        // The server's own refresh of the folder, as a real
                        // one may send; the client lists again anyway.
                        let mut r = vec![message(67, format!("Moved {name}").as_bytes())];
                        r.extend(dir(&cwd, &world[&cwd]));
                        r
                    }
                    97 => {
                        let name = text(p);
                        seen.push(format!("97 {name}"));
                        world.get_mut(&cwd).unwrap().remove(&name);
                        vec![message(67, format!("Deleted {name}").as_bytes())]
                    }
                    98 => {
                        let mut i = 0;
                        let old = text(&read_gstring(p, &mut i).unwrap());
                        let new = text(&read_gstring(p, &mut i).unwrap());
                        seen.push(format!("98 {old} {new}"));
                        let files = world.get_mut(&cwd).unwrap();
                        let data = files.remove(&old).unwrap();
                        files.insert(new, data);
                        vec![]
                    }
                    160 => {
                        let f = text(p);
                        seen.push(format!("160 {f}"));
                        // As the fork does: the folder goes, but its
                        // name still lists, empty, when the account's
                        // rights cover it (live run 2026-11-13).
                        let r = if f == "keep/" {
                            vec![message(
                                67,
                                format!("Could not remove folder {f}").as_bytes(),
                            )]
                        } else {
                            world.insert(f.clone(), BTreeMap::new());
                            vec![message(
                                67,
                                format!("Folder {f} has been removed.").as_bytes(),
                            )]
                        };
                        r
                    }
                    _ => continue,
                };
                if !out.is_empty() {
                    send(&mut s, &mut codec, &out);
                }
            }
        }
        (seen, world)
    });
    (addr, h)
}

fn world() -> World {
    let mut w = World::new();
    w.insert(
        "world/".into(),
        [
            ("a.nw".to_string(), b"GLEVNW01\n".to_vec()),
            ("b.nw".to_string(), b"B".to_vec()),
        ]
        .into(),
    );
    w.insert("old/".into(), BTreeMap::new());
    w
}

fn cli(addr: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_seihai-rc"))
        .arg("files")
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

fn ok(o: &Output) -> String {
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn ls_and_get_name_the_file_after_listing_its_folder() {
    let (addr, h) = fake(world());
    assert_eq!(ok(&cli(&addr, &["ls", "world/"])), "a.nw\nb.nw\n");
    h.join().unwrap();

    let (addr, h) = fake(world());
    let out = ok(&cli(&addr, &["get", "world/a.nw"]));
    assert_eq!(out, "GLEVNW01\n");
    let (seen, _) = h.join().unwrap();
    assert_eq!(seen, ["90 world/", "92 a.nw"]);
}

#[test]
fn put_then_lists_to_check() {
    let dir = std::env::temp_dir().join(format!("seihai-rc-put-{}", std::process::id()));
    std::fs::write(&dir, b"new\nlevel").unwrap();
    let (addr, h) = fake(world());
    let out = ok(&cli(&addr, &["put", dir.to_str().unwrap(), "world/c.nw"]));
    assert!(out.contains("sent 9 bytes as world/c.nw"), "{out}");
    let (seen, w) = h.join().unwrap();
    std::fs::remove_file(&dir).unwrap();
    assert_eq!(seen, ["90 world/", "93 c.nw", "90 world/"]);
    assert_eq!(w["world/"]["c.nw"], b"new\nlevel");
}

#[test]
fn mv_sends_96_and_checks_both_folders() {
    let (addr, h) = fake(world());
    let out = ok(&cli(&addr, &["mv", "world/a.nw", "old/"]));
    assert!(out.contains("[server] Moved a.nw"), "{out}");
    assert!(out.contains("moved world/a.nw to old/"), "{out}");
    let (seen, w) = h.join().unwrap();
    assert_eq!(seen, ["90 world/", "96 a.nw old/", "90 world/", "90 old/"]);
    assert!(w["old/"].contains_key("a.nw"));
}

#[test]
fn rename_sends_98_with_two_names() {
    let (addr, h) = fake(world());
    let out = ok(&cli(&addr, &["rename", "world/a.nw", "c.nw"]));
    assert!(out.contains("renamed world/a.nw to world/c.nw"), "{out}");
    let (seen, w) = h.join().unwrap();
    assert_eq!(seen, ["90 world/", "98 a.nw c.nw", "90 world/"]);
    assert!(w["world/"].contains_key("c.nw"));
}

#[test]
fn rm_needs_yes_then_sends_97() {
    let o = cli("127.0.0.1:1", &["rm", "world/a.nw"]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("--yes"));

    let (addr, h) = fake(world());
    let out = ok(&cli(&addr, &["rm", "world/a.nw", "--yes"]));
    assert!(out.contains("deleted world/a.nw"), "{out}");
    let (seen, w) = h.join().unwrap();
    assert_eq!(seen, ["90 world/", "97 a.nw", "90 world/"]);
    assert!(!w["world/"].contains_key("a.nw"));
}

#[test]
fn rm_of_a_file_not_listed_sends_nothing() {
    let (addr, h) = fake(world());
    let o = cli(&addr, &["rm", "world/zzz.nw", "--yes"]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("is not listed"));
    let (seen, _) = h.join().unwrap();
    assert_eq!(seen, ["90 world/"]);
}

#[test]
fn rmdir_sends_160_and_reads_the_servers_answer() {
    let (addr, h) = fake(world());
    let out = ok(&cli(&addr, &["rmdir", "old/", "--yes"]));
    assert!(out.contains("deleted folder old/"), "{out}");
    let (seen, w) = h.join().unwrap();
    assert_eq!(seen, ["90 old/", "160 old/"]);
    assert!(w["old/"].is_empty());
}

#[test]
fn rmdir_refused_by_the_server_fails() {
    let mut w = world();
    w.insert("keep/".into(), BTreeMap::new());
    let (addr, h) = fake(w);
    let o = cli(&addr, &["rmdir", "keep/", "--yes"]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("Could not remove folder keep/"));
    let (seen, _) = h.join().unwrap();
    assert_eq!(seen, ["90 keep/", "160 keep/"]);
}
