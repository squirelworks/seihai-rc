//! File link on an RC session: list a directory, fetch a file, send a file.
//!
//! The widths are the ones the gserver-v2 fork used on the loopback run of
//! 2026-11-01 (`docs/lab/2026-11-01-file-widths.md`). A level file contains
//! line feeds. [`crate::link::Link::send`] refuses a line feed inside an
//! ordinary message, so a file travels in a raw block.
//!
//! # Payload fields
//!
//! - List, id [`pli::RC_FILEBROWSER_CD`] (90): the folder is the rest of the
//!   line. There is no length byte. `""` sends an empty payload. The fork
//!   answers only when that text is a folder the account may open, and the
//!   folders it stores end in `/` when the right looks like `world/*`.
//! - Directory, id [`plo::RC_FILEBROWSER_DIR`] (66): a `GSTRING` folder, then
//!   one record per file. Each record is a space, a `GCHAR` length, a
//!   `GSTRING` name, a `GSTRING` rights text, a `GINT5` size and a `GINT5`
//!   modification time. Id 65 is not this reply.
//! - The line before that directory is id [`plo::RC_FILEBROWSER_MESSAGE`]
//!   (67), `Folder changed to ` and the folder. Any other id 67 is a refusal.
//! - Get, id [`pli::RC_FILEBROWSER_DOWN`] (92): the path is the rest of the
//!   line. There is no length byte. The fork joins it to the folder from the
//!   last list.
//! - File, id [`PLO_FILE`] (102), inside a raw block of id [`PLO_RAWDATA`]
//!   (100): a `GINT5` modification time, a `GCHAR` name length, the name, then
//!   the file bytes. The 2026-11-01 payload began `26 77 3a 62 7e 33` and then
//!   `onlinestartlocal.nw`: those six bytes are the time and the length
//!   (`0x33` is 19, the length of that name). There is no `GINT3` size. The
//!   raw block's length counts one line feed after the file; that byte is
//!   not part of the file. `split_messages` already unwraps id 100, so the
//!   session sees id 102.
//! - Put, id [`pli::RC_FILEBROWSER_UP`] (93), inside a raw block of id
//!   [`PLI_RAWDATA`] (50): a `GSTRING` name, then the file bytes. No size
//!   field. The fork answers id 67, `Uploaded file ` and the name. Any other
//!   id 67 is a refusal.
//!
//! - Move, id [`pli::RC_FILEBROWSER_MOVE`] (96): a `GSTRING` file name in the
//!   folder from the last list, then the target folder as the rest of the
//!   line. **check**
//! - Delete, id [`pli::RC_FILEBROWSER_DELETE`] (97): the file name in the
//!   folder from the last list, as the rest of the line. **check**
//! - Rename, id [`pli::RC_FILEBROWSER_RENAME`] (98): two `GSTRING`s, the old
//!   and the new name, both in the folder from the last list. **check**
//! - Folder delete, id [`pli::RC_FOLDERDELETE`] (160): the folder as the rest
//!   of the line. **check**
//!
//! The four layouts above are not captured from a real RC yet. The server's
//! answer to them is not relied on: [`Session::file_action`] returns the id
//! 67 lines it sends, and the command line lists the folder again to see
//! whether the change happened.
//!
//! Not sent: 89, 91, 155, 156. A file over [`MAX_FILE_BYTES`]
//! is refused. One frame length is a `u16`, and the large-file ids are how a
//! bigger file would travel once a lab run shows their layout.

use std::io;
use std::time::{Duration, Instant};

use crate::rc::{Event, Session};
use crate::wire::{gchar, latin1_bytes, message, push_gstring, read_gint5, read_gstring, text};

/// Client to server. Raw block: this id, a `GINT3` byte count, a line feed,
/// then that many bytes. The put message sits in one of these, because a
/// file may contain a line feed.
pub const PLI_RAWDATA: u8 = 50;

/// Server to client. The same header shape. `split_messages` keeps the
/// following bytes together, line feeds included, and returns the message
/// inside.
pub const PLO_RAWDATA: u8 = 100;

/// Server to client. The file inside a [`PLO_RAWDATA`] block.
pub const PLO_FILE: u8 = 102;

/// Largest file [`Session::put_file`] sends and [`Session::get_file`] accepts.
///
/// An incompressible file plus the raw-block header has to fit in one frame,
/// and a frame length is a `u16` (65535). 60 KiB leaves room for the header
/// and for zlib growing a file that does not compress.
pub const MAX_FILE_BYTES: usize = 60 * 1024;

/// Client to server file-browser ids. START, END and the large-file ids are
/// not sent.
pub mod pli {
    /// Not sent. Payload unchecked.
    pub const RC_FILEBROWSER_START: u8 = 89;
    /// List the folder named by the rest of the line.
    pub const RC_FILEBROWSER_CD: u8 = 90;
    /// Not sent. Payload unchecked.
    pub const RC_FILEBROWSER_END: u8 = 91;
    /// Fetch the file named by the rest of the line.
    pub const RC_FILEBROWSER_DOWN: u8 = 92;
    /// Send a file. The message sits inside [`super::PLI_RAWDATA`].
    pub const RC_FILEBROWSER_UP: u8 = 93;
    /// Move a file to another folder. See [`super::move_message`]. **check**
    pub const RC_FILEBROWSER_MOVE: u8 = 96;
    /// Delete a file. See [`super::delete_message`]. **check**
    pub const RC_FILEBROWSER_DELETE: u8 = 97;
    /// Rename a file. See [`super::rename_message`]. **check**
    pub const RC_FILEBROWSER_RENAME: u8 = 98;
    /// Not sent. Payload unchecked. For a file that does not fit in one frame.
    pub const RC_LARGEFILESTART: u8 = 155;
    /// Not sent. Payload unchecked.
    pub const RC_LARGEFILEEND: u8 = 156;
    /// Delete a folder. See [`super::folder_delete_message`]. **check**
    pub const RC_FOLDERDELETE: u8 = 160;
}

/// Server to client file-browser ids.
pub mod plo {
    /// Sent by id 89, which this client does not send. Not the directory.
    pub const RC_FILEBROWSER_DIRLIST: u8 = 65;
    /// The directory. See [`super::parse_dir`].
    pub const RC_FILEBROWSER_DIR: u8 = 66;
    /// A line of text. `Folder changed to ` and `Uploaded file ` are the
    /// answers to list and put. Any other text is a refusal.
    pub const RC_FILEBROWSER_MESSAGE: u8 = 67;
}

/// The id 67 line that comes before a directory.
const FOLDER_CHANGED: &str = "Folder changed to ";

/// The id 67 line that means a put was stored.
const UPLOADED: &str = "Uploaded file ";

/// Why a list, get or put failed.
#[derive(Debug)]
pub enum FileError {
    Io(io::Error),
    /// The server sent id 67, or closed the session with a reason.
    Refused(String),
    /// The link closed with no message.
    Closed,
    /// No file reply before the timeout.
    Timeout,
    /// A length ran past the payload, or a record did not match its length.
    Malformed(String),
    /// `len` bytes is over [`MAX_FILE_BYTES`].
    TooLarge {
        len: usize,
        max: usize,
    },
    /// The path is absolute, walks up with `..`, or is too long.
    InvalidPath(String),
}

impl FileError {
    fn malformed(msg: impl Into<String>) -> Self {
        FileError::Malformed(msg.into())
    }
}

impl std::fmt::Display for FileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileError::Io(e) => write!(f, "{e}"),
            FileError::Refused(why) => write!(f, "refused: {why}"),
            FileError::Closed => write!(f, "the server closed the link"),
            FileError::Timeout => write!(f, "no file reply from the server"),
            FileError::Malformed(why) => write!(f, "malformed file reply: {why}"),
            FileError::TooLarge { len, max } => {
                write!(f, "file is {len} bytes; the limit is {max}")
            }
            FileError::InvalidPath(why) => write!(f, "path: {why}"),
        }
    }
}

impl std::error::Error for FileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FileError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for FileError {
    fn from(e: io::Error) -> Self {
        FileError::Io(e)
    }
}

/// `GINT3`: three GCHARs, high 7 bits first. The raw-block header uses it.
fn push_gint3(out: &mut Vec<u8>, n: usize) {
    out.push(gchar(((n >> 14) & 0x7F) as u8));
    out.push(gchar(((n >> 7) & 0x7F) as u8));
    out.push(gchar((n & 0x7F) as u8));
}

/// Id, `GINT3` length, line feed, then `inner` (a message, id included).
fn raw_block(id: u8, inner: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(5 + inner.len());
    body.push(gchar(id));
    push_gint3(&mut body, inner.len());
    body.push(b'\n');
    body.extend_from_slice(inner);
    body
}

/// Relative `/` path as Latin-1, or an error. `allow_empty` is the list of
/// an empty folder name.
///
/// A single trailing `/` is kept: the fork's folder for a right such as
/// `world/*` is `world/`.
fn path_bytes(path: &str, allow_empty: bool) -> Result<Vec<u8>, FileError> {
    if path.is_empty() {
        return if allow_empty {
            Ok(Vec::new())
        } else {
            Err(FileError::InvalidPath("path is empty".into()))
        };
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return Err(FileError::InvalidPath(
            "path is absolute; the file link is relative to the world".into(),
        ));
    }
    let check = path.strip_suffix('/').unwrap_or(path);
    if check.is_empty() {
        return Err(FileError::InvalidPath(
            "path is absolute; the file link is relative to the world".into(),
        ));
    }
    for seg in check.split('/') {
        if seg.is_empty() || seg == ".." || seg.contains('\\') || seg.contains(':') {
            return Err(FileError::InvalidPath(format!(
                "path leaves the world or is not /-separated: {path}"
            )));
        }
    }
    let bytes = latin1_bytes(path);
    // Put's name is one GCHAR, so 223 is the longest name that can be sent.
    // List and get are the rest of a line; the same cap keeps one path check.
    if bytes.len() > 223 || bytes.contains(&b'\n') || bytes.contains(&0u8) {
        return Err(FileError::InvalidPath(
            "path is longer than 223 bytes".into(),
        ));
    }
    Ok(bytes)
}

/// List request. The folder is the rest of the line.
pub fn cd_message(dir: &str) -> Result<Vec<u8>, FileError> {
    let path = path_bytes(dir, true)?;
    Ok(message(pli::RC_FILEBROWSER_CD, &path))
}

/// Get request. The path is the rest of the line.
pub fn down_message(path: &str) -> Result<Vec<u8>, FileError> {
    let path = path_bytes(path, false)?;
    Ok(message(pli::RC_FILEBROWSER_DOWN, &path))
}

/// Put body, ready for [`crate::link::Link::send_body`].
///
/// The name is a `GSTRING`. The bytes that follow are the file. A file over
/// [`MAX_FILE_BYTES`] is refused and nothing is built for it.
pub fn upload_body(path: &str, data: &[u8]) -> Result<Vec<u8>, FileError> {
    if data.len() > MAX_FILE_BYTES {
        return Err(FileError::TooLarge {
            len: data.len(),
            max: MAX_FILE_BYTES,
        });
    }
    let path = path_bytes(path, false)?;
    let mut inner = Vec::with_capacity(2 + path.len() + data.len());
    inner.push(gchar(pli::RC_FILEBROWSER_UP));
    push_gstring(&mut inner, &path);
    inner.extend_from_slice(data);
    Ok(raw_block(PLI_RAWDATA, &inner))
}

/// A bare file name: one path segment, no `/`.
fn name_bytes(name: &str) -> Result<Vec<u8>, FileError> {
    if name.contains('/') {
        return Err(FileError::InvalidPath(format!(
            "{name}: give a name in the listed folder, not a path"
        )));
    }
    path_bytes(name, false)
}

/// Move: `GSTRING` name, then the target folder as the rest of the line.
/// **check**
pub fn move_message(name: &str, to_folder: &str) -> Result<Vec<u8>, FileError> {
    let mut p = Vec::new();
    push_gstring(&mut p, &name_bytes(name)?);
    p.extend(path_bytes(to_folder, true)?);
    Ok(message(pli::RC_FILEBROWSER_MOVE, &p))
}

/// Delete: the name as the rest of the line. **check**
pub fn delete_message(name: &str) -> Result<Vec<u8>, FileError> {
    Ok(message(pli::RC_FILEBROWSER_DELETE, &name_bytes(name)?))
}

/// Rename: `GSTRING` old name, `GSTRING` new name. **check**
pub fn rename_message(old: &str, new: &str) -> Result<Vec<u8>, FileError> {
    let mut p = Vec::new();
    push_gstring(&mut p, &name_bytes(old)?);
    push_gstring(&mut p, &name_bytes(new)?);
    Ok(message(pli::RC_FILEBROWSER_RENAME, &p))
}

/// Folder delete: the folder as the rest of the line. **check**
pub fn folder_delete_message(folder: &str) -> Result<Vec<u8>, FileError> {
    Ok(message(pli::RC_FOLDERDELETE, &path_bytes(folder, false)?))
}

/// `("world/", "a.nw")` for `world/a.nw`, `("", "a.nw")` for `a.nw`. The
/// fork joins a file name to the folder from the last list, so a command
/// lists the folder first and then names the file.
pub fn split_path(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(i) => (&path[..=i], &path[i + 1..]),
        None => ("", path),
    }
}

/// File names from a directory payload (id 66).
///
/// The folder name is skipped. A length that runs past the payload, or a
/// record that does not fill its `GCHAR` length, is an error.
pub fn parse_dir(payload: &[u8]) -> Result<Vec<String>, FileError> {
    let mut i = 0;
    read_gstring(payload, &mut i)
        .ok_or_else(|| FileError::malformed("directory folder length runs past the payload"))?;
    let mut names = Vec::new();
    while i < payload.len() {
        if payload[i] != b' ' {
            return Err(FileError::malformed(
                "directory entry does not start with a space",
            ));
        }
        i += 1;
        let n = payload
            .get(i)
            .ok_or_else(|| FileError::malformed("directory entry length runs past the payload"))?;
        let n = usize::from(n.wrapping_sub(0x20));
        i += 1;
        let Some(entry) = payload.get(i..i + n) else {
            return Err(FileError::malformed(
                "directory entry length runs past the payload",
            ));
        };
        i += n;
        let mut j = 0;
        let name = read_gstring(entry, &mut j)
            .ok_or_else(|| FileError::malformed("file name length runs past the entry"))?;
        read_gstring(entry, &mut j)
            .ok_or_else(|| FileError::malformed("rights length runs past the entry"))?;
        read_gint5(entry, &mut j)
            .ok_or_else(|| FileError::malformed("file size is shorter than a GINT5"))?;
        read_gint5(entry, &mut j)
            .ok_or_else(|| FileError::malformed("modification time is shorter than a GINT5"))?;
        if j != entry.len() {
            return Err(FileError::malformed("bytes left in a directory entry"));
        }
        names.push(text(&name));
    }
    Ok(names)
}

/// `(name, bytes)` from an id 102 payload.
///
/// The first five bytes are the modification time. The name is a `GSTRING`.
/// One trailing line feed is the raw block's extra byte and is dropped. A
/// file over [`MAX_FILE_BYTES`] is refused.
pub fn parse_file_body(payload: &[u8]) -> Result<(String, Vec<u8>), FileError> {
    let mut i = 0;
    read_gint5(payload, &mut i)
        .ok_or_else(|| FileError::malformed("file time is shorter than a GINT5"))?;
    let name = read_gstring(payload, &mut i)
        .ok_or_else(|| FileError::malformed("file name length runs past the payload"))?;
    let mut rest = payload.get(i..).unwrap_or_default();
    if let Some(body) = rest.strip_suffix(b"\n") {
        rest = body;
    }
    if rest.len() > MAX_FILE_BYTES {
        return Err(FileError::TooLarge {
            len: rest.len(),
            max: MAX_FILE_BYTES,
        });
    }
    Ok((text(&name), rest.to_vec()))
}

enum Step<T> {
    Done(T),
    Keep(Event),
    /// The event belongs to this call. It is not put back on the queue.
    Take,
    Stop(FileError),
}

impl Session {
    fn wait_until<T>(
        &mut self,
        deadline: Instant,
        mut step: impl FnMut(Event) -> Step<T>,
    ) -> Result<T, FileError> {
        let mut held = Vec::new();
        let result = loop {
            match self.next_event(deadline) {
                Ok(Some(ev)) => match step(ev) {
                    Step::Done(v) => break Ok(v),
                    Step::Stop(e) => break Err(e),
                    Step::Take => {}
                    Step::Keep(ev) => held.push(ev),
                },
                Ok(None) => {
                    break Err(if self.is_closed() {
                        FileError::Closed
                    } else {
                        FileError::Timeout
                    });
                }
                Err(e) => break Err(FileError::Io(e)),
            }
        };
        held.append(&mut self.pending);
        self.pending = held;
        result
    }

    /// The file names in `dir`, in the order the server sends them.
    ///
    /// `""` sends an empty folder name. Events that are not the directory
    /// (the login signature, chat, players) stay queued. The
    /// `Folder changed to ` line is consumed here.
    pub fn list_dir(&mut self, dir: &str, timeout: Duration) -> Result<Vec<String>, FileError> {
        self.link.send(&[cd_message(dir)?])?;
        let deadline = Instant::now() + timeout;
        self.wait_until(deadline, |ev| match ev {
            Event::Other(id, p) if id == plo::RC_FILEBROWSER_DIR => match parse_dir(&p) {
                Ok(names) => Step::Done(names),
                Err(e) => Step::Stop(e),
            },
            Event::Other(id, p) if id == plo::RC_FILEBROWSER_MESSAGE => {
                let msg = text(&p);
                if msg.starts_with(FOLDER_CHANGED) {
                    Step::Take
                } else {
                    Step::Stop(FileError::Refused(msg))
                }
            }
            Event::Disconnect(why) => Step::Stop(FileError::Refused(why)),
            other => Step::Keep(other),
        })
    }

    /// The bytes of `path`.
    ///
    /// The name beside the bytes is the file's own name, which on the
    /// 2026-11-01 run was the last path segment, so it is not compared with
    /// `path`. Other events stay queued.
    pub fn get_file(&mut self, path: &str, timeout: Duration) -> Result<Vec<u8>, FileError> {
        self.link.send(&[down_message(path)?])?;
        let deadline = Instant::now() + timeout;
        self.wait_until(deadline, |ev| match ev {
            Event::Other(id, p) if id == PLO_FILE => match parse_file_body(&p) {
                Ok((_name, bytes)) => Step::Done(bytes),
                Err(e) => Step::Stop(e),
            },
            Event::Other(id, p) if id == plo::RC_FILEBROWSER_MESSAGE => {
                Step::Stop(FileError::Refused(text(&p)))
            }
            Event::Disconnect(why) => Step::Stop(FileError::Refused(why)),
            other => Step::Keep(other),
        })
    }

    /// Send one move, delete, rename or folder-delete message, then read
    /// until nothing has come for `quiet` (or `deadline`). Returns the id 67
    /// lines the server sent; other events stay queued. What a successful
    /// action answers is not known yet (**check**), so the caller lists the
    /// folder again to see the result.
    pub fn file_action(
        &mut self,
        msg: Vec<u8>,
        quiet: Duration,
        deadline: Instant,
    ) -> Result<Vec<String>, FileError> {
        self.link.send(&[msg])?;
        let events = self.settle(quiet, deadline)?;
        let mut lines = Vec::new();
        let mut keep = Vec::new();
        for ev in events {
            match ev {
                Event::Other(id, p) if id == plo::RC_FILEBROWSER_MESSAGE => lines.push(text(&p)),
                // A directory the server sends by itself after the change is
                // not needed: the caller lists again.
                Event::Other(id, _) if id == plo::RC_FILEBROWSER_DIR => {}
                Event::Disconnect(why) => return Err(FileError::Refused(why)),
                other => keep.push(other),
            }
        }
        keep.append(&mut self.pending);
        self.pending = keep;
        Ok(lines)
    }

    /// Send `data` as `path`, and wait until the server answers.
    ///
    /// A file over [`MAX_FILE_BYTES`] is refused and nothing is sent. Id 67
    /// beginning `Uploaded file ` is success. Any other id 67 is a refusal.
    pub fn put_file(
        &mut self,
        path: &str,
        data: &[u8],
        timeout: Duration,
    ) -> Result<(), FileError> {
        self.link.send_body(&upload_body(path, data)?)?;
        let deadline = Instant::now() + timeout;
        self.wait_until(deadline, |ev| match ev {
            Event::Other(id, p) if id == plo::RC_FILEBROWSER_MESSAGE => {
                let msg = text(&p);
                if msg.starts_with(UPLOADED) {
                    Step::Done(())
                } else {
                    Step::Stop(FileError::Refused(msg))
                }
            }
            Event::Disconnect(why) => Step::Stop(FileError::Refused(why)),
            other => Step::Keep(other),
        })
    }
}

#[cfg(test)]
mod tests {
    use seihai_core::protocol::split_messages;

    use super::*;
    use crate::wire::push_gint5;

    #[test]
    fn cd_and_down_have_no_length_byte() {
        assert_eq!(cd_message("levels").unwrap(), b"zlevels");
        assert_eq!(cd_message("").unwrap(), b"z");
        assert_eq!(cd_message("world/").unwrap(), b"zworld/");
        assert_eq!(down_message("a.nw").unwrap(), b"|a.nw");
        assert_eq!(
            down_message("world/onlinestartlocal.nw").unwrap(),
            b"|world/onlinestartlocal.nw"
        );
    }

    #[test]
    fn move_delete_rename_and_folder_delete_bytes() {
        // 96: GSTRING name, then the folder as the rest of the line.
        assert_eq!(move_message("a.nw", "old/").unwrap(), b"\x80$a.nwold/");
        // 97: the name as the rest of the line.
        assert_eq!(delete_message("a.nw").unwrap(), b"\x81a.nw");
        // 98: two GSTRINGs.
        assert_eq!(rename_message("a.nw", "b.nw").unwrap(), b"\x82$a.nw$b.nw");
        // 160: the folder as the rest of the line.
        assert_eq!(folder_delete_message("old/").unwrap(), b"\xc0old/");
        assert!(delete_message("world/a.nw").is_err());
        assert!(rename_message("a.nw", "../b.nw").is_err());
        assert!(move_message("a.nw", "../x").is_err());
        assert!(folder_delete_message("").is_err());
    }

    #[test]
    fn split_path_keeps_the_folder_slash() {
        assert_eq!(split_path("world/a.nw"), ("world/", "a.nw"));
        assert_eq!(split_path("a.nw"), ("", "a.nw"));
        assert_eq!(split_path("w/x/a.nw"), ("w/x/", "a.nw"));
    }

    #[test]
    fn upload_bytes_are_the_file_after_the_name() {
        // Id 50, GINT3(9), line feed, then id 93, GSTRING "a.nw", "hi\n".
        // No GINT3 size in front of the file.
        assert_eq!(upload_body("a.nw", b"hi\n").unwrap(), b"R  )\n}$a.nwhi\n");
    }

    #[test]
    fn inbound_file_with_a_line_feed_survives_split_messages() {
        let data = b"GLEVNW01\n\xff";
        let mut packet = vec![gchar(PLO_FILE)];
        push_gint5(&mut packet, 0);
        push_gstring(&mut packet, b"a.nw");
        packet.extend_from_slice(data);
        // The raw length counts one line feed after the packet.
        let n = packet.len() + 1;
        let mut body = vec![gchar(PLO_RAWDATA)];
        push_gint3(&mut body, n);
        body.push(b'\n');
        body.extend_from_slice(&packet);
        body.push(b'\n');
        let msgs = split_messages(&body);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].id, PLO_FILE);
        let (name, bytes) = parse_file_body(&msgs[0].payload).unwrap();
        assert_eq!(name, "a.nw");
        assert_eq!(bytes, data);
    }

    #[test]
    fn lab_file_prefix_is_the_time_and_the_name_length() {
        // 2026-11-01: six bytes, then `onlinestartlocal.nw`, then the level.
        // 0x33 is GCHAR(19), and that name is 19 bytes.
        let mut p = vec![0x26, 0x77, 0x3a, 0x62, 0x7e, 0x33];
        p.extend_from_slice(b"onlinestartlocal.nw");
        p.extend_from_slice(b"GLEVNW01\n");
        p.push(b'\n');
        let (name, bytes) = parse_file_body(&p).unwrap();
        assert_eq!(name, "onlinestartlocal.nw");
        assert_eq!(bytes, b"GLEVNW01\n");
    }

    #[test]
    fn dir_lists_the_names_and_an_empty_folder() {
        // Folder "a", one file "b", rights "r", size 0, time 0.
        // Record is 14 bytes, so the entry is a space, '.', then the record.
        let mut p = b"!a .!b!r".to_vec();
        p.extend_from_slice(&[b' '; 10]);
        assert_eq!(parse_dir(&p).unwrap(), ["b"]);
        assert!(parse_dir(b"!a").unwrap().is_empty());

        let mut two = Vec::new();
        push_gstring(&mut two, b"levels");
        for name in [b"a.nw".as_slice(), b"\xE9"] {
            let mut rec = Vec::new();
            push_gstring(&mut rec, name);
            push_gstring(&mut rec, b"rw");
            push_gint5(&mut rec, 3);
            push_gint5(&mut rec, 1);
            two.push(b' ');
            two.push(gchar(rec.len() as u8));
            two.extend_from_slice(&rec);
        }
        assert_eq!(parse_dir(&two).unwrap(), ["a.nw", "é"]);
    }

    #[test]
    fn malformed_lengths_are_errors() {
        assert!(parse_dir(b"").is_err());
        // Folder length 10, five bytes follow.
        assert!(matches!(
            parse_dir(&[gchar(10), b's', b'h', b'o', b'r', b't']),
            Err(FileError::Malformed(_))
        ));
        // A space, then a record length that runs past the payload.
        let mut p = b"!a".to_vec();
        p.push(b' ');
        p.push(gchar(10));
        p.push(b'x');
        assert!(matches!(parse_dir(&p), Err(FileError::Malformed(_))));
        assert!(parse_file_body(b"").is_err());
        // Time is five bytes. Four bytes is not a file.
        assert!(matches!(
            parse_file_body(&[0x20, 0x20, 0x20, 0x20]),
            Err(FileError::Malformed(_))
        ));
        // Time, then a name length that runs past the payload.
        let mut p = vec![0x20, 0x20, 0x20, 0x20, 0x20, gchar(4)];
        p.extend_from_slice(b"ab");
        assert!(matches!(parse_file_body(&p), Err(FileError::Malformed(_))));
    }

    #[test]
    fn too_large_file_is_refused() {
        let big = vec![0u8; MAX_FILE_BYTES + 1];
        assert!(matches!(
            upload_body("a.nw", &big),
            Err(FileError::TooLarge { len, max }) if len == big.len() && max == MAX_FILE_BYTES
        ));
        let mut p = vec![0x20, 0x20, 0x20, 0x20, 0x20];
        push_gstring(&mut p, b"a");
        p.extend_from_slice(&big);
        assert!(matches!(
            parse_file_body(&p),
            Err(FileError::TooLarge { len, .. }) if len == MAX_FILE_BYTES + 1
        ));
    }

    #[test]
    fn paths_outside_the_world_are_refused() {
        for bad in ["../x", "/outside/a.nw", "a/../../b", "a\\b", "C:/a.nw", ""] {
            assert!(down_message(bad).is_err(), "{bad}");
            assert!(upload_body(bad, b"z").is_err(), "{bad}");
        }
        assert!(cd_message("../x").is_err());
        assert!(cd_message("/world/").is_err());
        assert!(cd_message("a//b").is_err());
        assert!(cd_message("").is_ok());
        assert!(cd_message("world/").is_ok());
        assert!(down_message("world/a.nw").is_ok());
    }

    #[test]
    fn empty_file_round_trip() {
        let mut p = Vec::new();
        push_gint5(&mut p, 0);
        push_gstring(&mut p, b"a.nw");
        p.push(b'\n');
        let (name, bytes) = parse_file_body(&p).unwrap();
        assert_eq!(name, "a.nw");
        assert!(bytes.is_empty());
    }
}
