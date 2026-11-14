//! A live NPC-Control session on its own thread, for the window's NPC
//! Control tab. The same shape as [`crate::live`]: [`LiveNc`] takes
//! [`NcCommand`]s and sends [`NcUpdate`]s, and [`NcView`] is what the
//! window draws, built only from updates (no egui in it).

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::nc::{
    class_delete_message, class_get_message, class_set_message, matching_script,
    npc_delete_message, npc_get_message, npc_reset_message, npc_script_get_message,
    npc_script_set_message, rc_chat_refusal, representable_npc_class_script,
    representable_weapon_script, script_subject_phrase, weapon_delete_message, weapon_get_message,
    weapon_list_message, weapon_set_message, NcError, NcEvent, NcLists, NcSession,
    ScriptObservation, ScriptSubject,
};

const POLL: Duration = Duration::from_millis(50);

/// Something the person asked the NPC-Control session to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NcCommand {
    /// NPC attributes (103 → 157).
    NpcAttributes(u32),
    /// NPC script (106 → 160).
    NpcScript(u32),
    /// Save an NPC script (109).
    NpcScriptSet { id: u32, script: String },
    /// Delete an NPC (104 → 159).
    NpcDelete(u32),
    /// Reset an NPC (105 → 29).
    NpcReset(u32),
    /// Open a class (112 → 162).
    ClassGet(String),
    /// Save a class (113).
    ClassSet { name: String, script: String },
    /// Delete a class (119 → 188).
    ClassDelete(String),
    /// Ask for the weapon list (115 → 167).
    Weapons,
    /// Open a weapon (116 → 33 or 192).
    WeaponGet(String),
    /// Save a weapon (117), then ask for the list again.
    WeaponSet {
        name: String,
        image: String,
        script: String,
    },
    /// Delete a weapon (118), then ask for the list again.
    WeaponDelete(String),
    /// Log out and end the worker.
    Quit,
}

/// Something the NPC-Control worker tells the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NcUpdate {
    Connected(String),
    LoginFailed(String),
    Event(NcEvent),
    Sent(String),
    Error(String),
    Closed(String),
}

/// The messages for a command and the log line for it (empty for reads).
pub type Encoded = Result<(Vec<Vec<u8>>, String), NcError>;

/// The bytes for a command and the log line for it; `None` for quit.
pub fn encode(c: &NcCommand) -> Option<Encoded> {
    let one = |m: Result<Vec<u8>, NcError>, note: String| m.map(|m| (vec![m], note));
    Some(match c {
        NcCommand::NpcAttributes(id) => one(npc_get_message(*id), String::new()),
        NcCommand::NpcScript(id) => one(npc_script_get_message(*id), String::new()),
        NcCommand::NpcScriptSet { id, script } => npc_script_set_message(*id, script)
            .and_then(|set| npc_script_get_message(*id).map(|get| (vec![set, get], String::new()))),
        NcCommand::NpcDelete(id) => one(npc_delete_message(*id), format!("deleted NPC {id}")),
        NcCommand::NpcReset(id) => one(npc_reset_message(*id), format!("reset NPC {id}")),
        NcCommand::ClassGet(name) => one(class_get_message(name), String::new()),
        NcCommand::ClassSet { name, script } => class_set_message(name, script)
            .and_then(|set| class_get_message(name).map(|get| (vec![set, get], String::new()))),
        NcCommand::ClassDelete(name) => {
            one(class_delete_message(name), format!("deleted class {name}"))
        }
        NcCommand::Weapons => Ok((vec![weapon_list_message()], String::new())),
        NcCommand::WeaponGet(name) => one(weapon_get_message(name), String::new()),
        NcCommand::WeaponSet {
            name,
            image,
            script,
        } => weapon_set_message(name, image, script).and_then(|set| {
            weapon_get_message(name)
                .map(|get| (vec![set, get, weapon_list_message()], String::new()))
        }),
        NcCommand::WeaponDelete(name) => weapon_delete_message(name).map(|m| {
            (
                vec![m, weapon_list_message()],
                format!("deleted weapon {name}"),
            )
        }),
        NcCommand::Quit => return None,
    })
}

/// The worker thread and its channels.
pub struct LiveNc {
    commands: Sender<NcCommand>,
    updates: Receiver<NcUpdate>,
    worker: Option<JoinHandle<()>>,
}

impl LiveNc {
    /// Log in to the NPC-server at `addr` on a new thread, then ask for the
    /// weapon list (the NPCs and classes come by themselves).
    pub fn start(addr: String, account: String, password: String, timeout: Duration) -> Self {
        let (ctx, crx) = mpsc::channel();
        let (utx, urx) = mpsc::channel();
        let worker = thread::spawn(move || run(addr, account, password, timeout, crx, utx));
        Self {
            commands: ctx,
            updates: urx,
            worker: Some(worker),
        }
    }

    pub fn send(&self, c: NcCommand) -> bool {
        self.commands.send(c).is_ok()
    }

    pub fn drain(&self) -> Vec<NcUpdate> {
        let mut out = Vec::new();
        while let Ok(u) = self.updates.try_recv() {
            out.push(u);
        }
        out
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Option<NcUpdate> {
        self.updates.recv_timeout(timeout).ok()
    }
}

impl Drop for LiveNc {
    fn drop(&mut self) {
        let _ = self.commands.send(NcCommand::Quit);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

/// A script get that has been sent and not answered yet.
struct OpenScriptRead {
    subject: ScriptSubject,
    /// Set when this get follows a save. `None` is an ordinary open.
    expected: Option<String>,
}

fn command_subject(c: &NcCommand) -> Option<ScriptSubject> {
    match c {
        NcCommand::NpcScript(id) | NcCommand::NpcScriptSet { id, .. } => {
            Some(ScriptSubject::Npc(*id))
        }
        NcCommand::ClassGet(name) | NcCommand::ClassSet { name, .. } => {
            Some(ScriptSubject::Class(name.clone()))
        }
        NcCommand::WeaponGet(name) | NcCommand::WeaponSet { name, .. } => {
            Some(ScriptSubject::Weapon(name.clone()))
        }
        _ => None,
    }
}

fn save_expectation(c: &NcCommand) -> Option<(ScriptSubject, String)> {
    match c {
        NcCommand::NpcScriptSet { id, script } => Some((
            ScriptSubject::Npc(*id),
            representable_npc_class_script(script),
        )),
        NcCommand::ClassSet { name, script } => Some((
            ScriptSubject::Class(name.clone()),
            representable_npc_class_script(script),
        )),
        NcCommand::WeaponSet { name, script, .. } => Some((
            ScriptSubject::Weapon(name.clone()),
            representable_weapon_script(script),
        )),
        _ => None,
    }
}

/// A save or an open while a read for that same target is already out.
fn blocked_read(reads: &[OpenScriptRead], c: &NcCommand) -> Option<String> {
    let subject = command_subject(c)?;
    reads.iter().any(|read| read.subject == subject).then(|| {
        format!(
            "a script read for {} is already waiting",
            script_subject_phrase(&subject)
        )
    })
}

fn verdict(line: Result<String, String>) -> NcUpdate {
    match line {
        Ok(line) => NcUpdate::Sent(line),
        Err(error) => NcUpdate::Error(error),
    }
}

/// Decide any save that this one event settles. An ordinary open is only
/// cleared, so the editor can show the script without a save line.
fn judge(reads: &mut Vec<OpenScriptRead>, ev: &NcEvent) -> Vec<NcUpdate> {
    if let Some(index) = reads
        .iter()
        .position(|read| matching_script(&read.subject, ev).is_some())
    {
        let read = reads.remove(index);
        let Some(expected) = read.expected else {
            return Vec::new();
        };
        let got = matching_script(&read.subject, ev).unwrap_or_default();
        let observation = if got == expected {
            ScriptObservation::Observed
        } else {
            ScriptObservation::Different {
                got: got.to_string(),
            }
        };
        return vec![verdict(
            observation.outcome_line(&read.subject, expected.len()),
        )];
    }
    if let NcEvent::Disconnect(why) = ev {
        let saves: Vec<_> = reads
            .drain(..)
            .filter(|read| read.expected.is_some())
            .collect();
        return saves
            .iter()
            .filter_map(|read| {
                ScriptObservation::Disconnected(why.clone())
                    .outcome_line(&read.subject, 0)
                    .err()
                    .map(NcUpdate::Error)
            })
            .collect();
    }
    if let Some(why) = rc_chat_refusal(ev) {
        let mut saves = Vec::new();
        let mut still_open = Vec::new();
        for read in reads.drain(..) {
            if read.expected.is_some() {
                saves.push(read);
            } else {
                still_open.push(read);
            }
        }
        *reads = still_open;
        return saves
            .iter()
            .filter_map(|read| {
                ScriptObservation::Refused(why.clone())
                    .outcome_line(&read.subject, 0)
                    .err()
                    .map(NcUpdate::Error)
            })
            .collect();
    }
    Vec::new()
}

fn run(
    addr: String,
    account: String,
    password: String,
    timeout: Duration,
    commands: Receiver<NcCommand>,
    updates: Sender<NcUpdate>,
) {
    let mut s = match NcSession::login(&addr, &account, &password, timeout) {
        Ok(s) => s,
        Err(e) => {
            let _ = updates.send(NcUpdate::LoginFailed(format!("{addr}: {e}")));
            return;
        }
    };
    if updates.send(NcUpdate::Connected(addr)).is_err() {
        s.close();
        return;
    }
    if let Err(e) = s.send(&[weapon_list_message()]) {
        let _ = updates.send(NcUpdate::Error(format!("send failed: {e}")));
    }
    let mut reads: Vec<OpenScriptRead> = Vec::new();
    let why = loop {
        let mut quit = false;
        loop {
            match commands.try_recv() {
                Ok(c) => match encode(&c) {
                    None => quit = true,
                    Some(Err(e)) => {
                        let _ = updates.send(NcUpdate::Error(e.to_string()));
                    }
                    Some(Ok((msgs, note))) => {
                        if let Some(why) = blocked_read(&reads, &c) {
                            let _ = updates.send(NcUpdate::Error(why));
                        } else {
                            match s.send(&msgs) {
                                Ok(()) => {
                                    if let Some((subject, expected)) = save_expectation(&c) {
                                        let phrase = script_subject_phrase(&subject);
                                        reads.push(OpenScriptRead {
                                            subject,
                                            expected: Some(expected),
                                        });
                                        let _ = updates.send(NcUpdate::Sent(format!(
                                            "asked to read {phrase} back"
                                        )));
                                    } else if let Some(subject) = command_subject(&c) {
                                        reads.push(OpenScriptRead {
                                            subject,
                                            expected: None,
                                        });
                                    } else if !note.is_empty() {
                                        let _ = updates.send(NcUpdate::Sent(note));
                                    }
                                }
                                Err(e) => {
                                    let _ =
                                        updates.send(NcUpdate::Error(format!("send failed: {e}")));
                                }
                            }
                        }
                    }
                },
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
                let end = matches!(ev, NcEvent::Disconnect(_));
                let extra = judge(&mut reads, &ev);
                let mut closed = updates.send(NcUpdate::Event(ev)).is_err();
                for update in extra {
                    closed |= updates.send(update).is_err();
                }
                if closed {
                    break "window closed".to_string();
                }
                if end {
                    break "the NPC-server ended the session".to_string();
                }
            }
            Ok(None) if s.is_closed() => break "the NPC-server closed the link".to_string(),
            Ok(None) => {}
            Err(e) => break format!("link error: {e}"),
        }
    };
    s.close();
    let _ = updates.send(NcUpdate::Closed(why));
}

/// What the script editor holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptTarget {
    Npc(u32),
    Class(String),
    Weapon { name: String, image: String },
}

/// NPC Control as the window draws it.
#[derive(Debug, Default)]
pub struct NcView {
    /// The address while connected.
    pub online: Option<String>,
    /// Why the last session ended or failed.
    pub ended: Option<String>,
    pub lists: NcLists,
    /// The last NPC attributes read (157), as lines.
    pub attributes: Option<String>,
    /// The script that arrived last, and what it belongs to.
    pub script: Option<(ScriptTarget, String)>,
    pub log: Vec<String>,
}

impl NcView {
    pub fn apply(&mut self, u: NcUpdate) {
        match u {
            NcUpdate::Connected(addr) => {
                self.log.push(format!("[nc connected] {addr}"));
                self.online = Some(addr);
                self.ended = None;
            }
            NcUpdate::LoginFailed(why) => {
                self.log.push(format!("[nc login failed] {why}"));
                self.ended = Some(why);
            }
            NcUpdate::Event(ev) => {
                self.lists.apply(&ev);
                match ev {
                    NcEvent::Attributes(a) => self.attributes = Some(a),
                    NcEvent::NpcScript { id, script } => {
                        self.script = Some((ScriptTarget::Npc(id), script));
                    }
                    NcEvent::ClassScript { name, script } => {
                        self.script = Some((ScriptTarget::Class(name), script));
                    }
                    NcEvent::Weapon(w) => {
                        self.script = Some((
                            ScriptTarget::Weapon {
                                name: w.name,
                                image: w.image,
                            },
                            w.script,
                        ));
                    }
                    NcEvent::NpcReset(id) => self.log.push(format!("[nc] NPC {id} was reset")),
                    NcEvent::NpcRemoved(id) => self.log.push(format!("[nc] NPC {id} removed")),
                    NcEvent::ClassDeleted(n) => self.log.push(format!("[nc] class {n} deleted")),
                    NcEvent::Disconnect(why) => self.log.push(format!("[nc disconnected] {why}")),
                    _ => {}
                }
            }
            NcUpdate::Sent(note) => self.log.push(format!("[nc sent] {note}")),
            NcUpdate::Error(e) => self.log.push(format!("[nc error] {e}")),
            NcUpdate::Closed(why) => {
                self.log.push(format!("[nc closed] {why}"));
                self.online = None;
                self.ended = Some(why);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nc::{NpcRow, WeaponScript};

    #[test]
    fn view_follows_lists_and_scripts() {
        let mut v = NcView::default();
        v.apply(NcUpdate::Connected("127.0.0.1:1".into()));
        v.apply(NcUpdate::Event(NcEvent::Npc(NpcRow {
            id: 1001,
            name: Some("door".into()),
            kind: Some("LOCALN".into()),
            level: Some("start.nw".into()),
        })));
        v.apply(NcUpdate::Event(NcEvent::Class("shop".into())));
        v.apply(NcUpdate::Event(NcEvent::Weapons(vec!["bomb".into()])));
        v.apply(NcUpdate::Event(NcEvent::Weapon(WeaponScript {
            name: "bomb".into(),
            image: "wbomb1.png".into(),
            script: "//#CLIENTSIDE".into(),
        })));
        assert_eq!(v.lists.npcs.len(), 1);
        assert_eq!(v.lists.classes, vec!["shop"]);
        assert_eq!(v.lists.weapons, vec!["bomb"]);
        assert_eq!(
            v.script,
            Some((
                ScriptTarget::Weapon {
                    name: "bomb".into(),
                    image: "wbomb1.png".into()
                },
                "//#CLIENTSIDE".into()
            ))
        );
        v.apply(NcUpdate::Event(NcEvent::ClassDeleted("shop".into())));
        assert!(v.lists.classes.is_empty());
        v.apply(NcUpdate::Closed("logged out".into()));
        assert_eq!(v.online, None);
    }

    #[test]
    fn weapon_set_asks_for_the_list_again() {
        let (msgs, note) = encode(&NcCommand::WeaponSet {
            name: "bomb".into(),
            image: "wbomb1.png".into(),
            script: "a".into(),
        })
        .unwrap()
        .unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[1], weapon_get_message("bomb").unwrap());
        assert_eq!(msgs[2], weapon_list_message());
        assert!(note.is_empty());
        let (npc, npc_note) = encode(&NcCommand::NpcScriptSet {
            id: 12,
            script: "return;".into(),
        })
        .unwrap()
        .unwrap();
        assert!(npc_note.is_empty());
        assert_eq!(npc[1], npc_script_get_message(12).unwrap());
        let (class, class_note) = encode(&NcCommand::ClassSet {
            name: "Guard".into(),
            script: "return;".into(),
        })
        .unwrap()
        .unwrap();
        assert!(class_note.is_empty());
        assert_eq!(class[1], class_get_message("Guard").unwrap());
        assert!(encode(&NcCommand::Quit).is_none());
    }

    #[test]
    fn a_second_read_for_the_same_script_waits() {
        let pending = vec![OpenScriptRead {
            subject: ScriptSubject::Npc(7),
            expected: None,
        }];
        let again = NcCommand::NpcScriptSet {
            id: 7,
            script: "a".into(),
        };
        assert!(blocked_read(&pending, &again)
            .unwrap()
            .contains("already waiting"));
        let other = NcCommand::ClassSet {
            name: "shop".into(),
            script: "a".into(),
        };
        assert!(blocked_read(&pending, &other).is_none());
    }
}
