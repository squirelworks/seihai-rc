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
//! - Move, id [`pli::RC_FILEBROWSER_MOVE`] (96): a `GSTRING` target folder,
//!   then the file name in the folder from the last list as the rest of the
//!   line. seihai-gserver-v2 reads this order (lab run 2026-11-12).
//! - Delete, id [`pli::RC_FILEBROWSER_DELETE`] (97): the file name in the
//!   folder from the last list, as the rest of the line.
//! - Rename, id [`pli::RC_FILEBROWSER_RENAME`] (98): two `GSTRING`s, the old
//!   and the new name, both in the folder from the last list.
//! - Folder delete, id [`pli::RC_FOLDERDELETE`] (160): the folder and a
//!   trailing `/` as the rest of the line.
//!
//! All four match real RCs (the 2004 RC and RC3, lab 2026-11-13). The
//! [`Session::file_action`] returns the id 67 lines the server sends. For
//! move, delete and rename the command line lists the folder again to see
//! whether the change happened; for folder delete it reads the 67 line,
//! because the fork still lists a deleted folder the account's rights cover.
//!
//! # Large files
//!
//! One frame length is a `u16`, so a file over [`MAX_FILE_BYTES`] travels in
//! pieces. The layouts match RC3's put of 81200 bytes and the 2004 RC's get
//! of 50000 bytes (lab 2026-11-13).
//!
//! - Put: id [`pli::RC_LARGEFILESTART`] (155) with the name as the rest of
//!   the line, then the file as ordinary put blocks (id 93 inside id 50) of at
//!   most [`LARGE_CHUNK_BYTES`] each, all with the same name, then id
//!   [`pli::RC_LARGEFILEEND`] (156) with the name as the rest of the line.
//!   The fork keeps the pieces for that name and joins them; it does not
//!   answer a piece. On 156 it stores the file in the folder from the last
//!   list and answers id 67, `Uploaded large file ` and the name.
//! - Get: the server sends id [`plo::LARGEFILESTART`] (68) with the name as
//!   the rest of the line, then id [`plo::LARGEFILESIZE`] (84), a `GINT5`
//!   total size (2.3 and later; an older server leaves it out), then the
//!   file as id 102 blocks, then id [`plo::LARGEFILEEND`] (69) with the name.
//!   The fork sends a file over 32000 bytes this way, so a file between that
//!   and [`MAX_FILE_BYTES`] arrives in pieces too.
//!
//! A file over [`MAX_LARGE_FILE_BYTES`] is refused either way.
//!
//! Not sent: 89, 91.

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

/// Bytes of the file in each piece of a large put. The fork sends pieces of
/// this size, and it leaves room in a frame like [`MAX_FILE_BYTES`] does.
pub const LARGE_CHUNK_BYTES: usize = 32_000;

/// Largest file sent or accepted as a large file. The whole file is held in
/// memory, so a server that announces more is refused before any piece is
/// kept.
pub const MAX_LARGE_FILE_BYTES: usize = 64 * 1024 * 1024;

/// Client to server file-browser ids. START and END are not sent.
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
    /// Move a file to another folder. See [`super::move_message`].
    pub const RC_FILEBROWSER_MOVE: u8 = 96;
    /// Delete a file. See [`super::delete_message`].
    pub const RC_FILEBROWSER_DELETE: u8 = 97;
    /// Rename a file. See [`super::rename_message`].
    pub const RC_FILEBROWSER_RENAME: u8 = 98;
    /// A large put starts. The name is the rest of the line.
    pub const RC_LARGEFILESTART: u8 = 155;
    /// A large put ends and the server stores it. The name is the rest of
    /// the line.
    pub const RC_LARGEFILEEND: u8 = 156;
    /// Delete a folder. See [`super::folder_delete_message`].
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
    /// A large file starts. The name is the rest of the line. Server to
    /// client only: the same id from the client is the server-flags get.
    pub const LARGEFILESTART: u8 = 68;
    /// A large file is complete. The name is the rest of the line.
    pub const LARGEFILEEND: u8 = 69;
    /// The total size of the large file, a `GINT5`. Sent after
    /// [`LARGEFILESTART`] by 2.3 and later servers.
    pub const LARGEFILESIZE: u8 = 84;
}

/// The id 67 line that comes before a directory.
const FOLDER_CHANGED: &str = "Folder changed to ";

/// The id 67 line that means a put was stored.
const UPLOADED: &str = "Uploaded file ";

/// The id 67 line that means a large put was stored.
const UPLOADED_LARGE: &str = "Uploaded large file ";

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
    /// `len` bytes is over [`MAX_FILE_BYTES`] for a single block, or over
    /// [`MAX_LARGE_FILE_BYTES`] for a large file.
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

/// Id, `GINT3` length, line feed, then `inner` (a message, id included)
/// and its own line feed. The length counts that last line feed, as real
/// RCs count it (the 2004 RC and RC3, lab 2026-11-13: 32015 for a
/// 32014-byte id 93). The server drops it; a length without it made the
/// fork drop the file's own last byte instead (seihai-rc live run,
/// 2026-11-13).
fn raw_block(id: u8, inner: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(6 + inner.len());
    body.push(gchar(id));
    push_gint3(&mut body, inner.len() + 1);
    body.push(b'\n');
    body.extend_from_slice(inner);
    body.push(b'\n');
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
    Ok(put_block(&path, data))
}

/// Id 93 inside an id 50 raw block: `GSTRING` name, then `data`.
fn put_block(path: &[u8], data: &[u8]) -> Vec<u8> {
    let mut inner = Vec::with_capacity(2 + path.len() + data.len());
    inner.push(gchar(pli::RC_FILEBROWSER_UP));
    push_gstring(&mut inner, path);
    inner.extend_from_slice(data);
    raw_block(PLI_RAWDATA, &inner)
}

/// A large put as frame bodies, in the order they are sent: id 155 with
/// the name, one put block per [`LARGE_CHUNK_BYTES`] of the file, then id
/// 156 with the name. Each body is ready for
/// [`crate::link::Link::send_body`]. **check**
///
/// A file over [`MAX_LARGE_FILE_BYTES`] is refused and nothing is built.
pub fn large_upload_bodies(path: &str, data: &[u8]) -> Result<Vec<Vec<u8>>, FileError> {
    if data.len() > MAX_LARGE_FILE_BYTES {
        return Err(FileError::TooLarge {
            len: data.len(),
            max: MAX_LARGE_FILE_BYTES,
        });
    }
    let path = path_bytes(path, false)?;
    let line = |id| {
        let mut m = message(id, &path);
        m.push(b'\n');
        m
    };
    let mut bodies = vec![line(pli::RC_LARGEFILESTART)];
    bodies.extend(
        data.chunks(LARGE_CHUNK_BYTES)
            .map(|piece| put_block(&path, piece)),
    );
    bodies.push(line(pli::RC_LARGEFILEEND));
    Ok(bodies)
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

/// Move: `GSTRING` target folder, then the file name (in the folder from
/// the last list) as the rest of the line. The order seihai-gserver-v2 reads
/// (lab run 2026-11-12); the earlier reading (name first) was refused there.
pub fn move_message(name: &str, to_folder: &str) -> Result<Vec<u8>, FileError> {
    let mut p = Vec::new();
    push_gstring(&mut p, &path_bytes(to_folder, true)?);
    p.extend(name_bytes(name)?);
    Ok(message(pli::RC_FILEBROWSER_MOVE, &p))
}

/// Delete: the name as the rest of the line.
pub fn delete_message(name: &str) -> Result<Vec<u8>, FileError> {
    Ok(message(pli::RC_FILEBROWSER_DELETE, &name_bytes(name)?))
}

/// Rename: `GSTRING` old name, `GSTRING` new name.
pub fn rename_message(old: &str, new: &str) -> Result<Vec<u8>, FileError> {
    let mut p = Vec::new();
    push_gstring(&mut p, &name_bytes(old)?);
    push_gstring(&mut p, &name_bytes(new)?);
    Ok(message(pli::RC_FILEBROWSER_RENAME, &p))
}

/// Folder delete: the folder with a trailing `/`, as the rest of the line.
/// RC3 sent `world/labgone/` from its folder menu (lab 2026-11-13); a
/// folder given without the slash gets one.
pub fn folder_delete_message(folder: &str) -> Result<Vec<u8>, FileError> {
    let mut bytes = path_bytes(folder, false)?;
    if bytes.last() != Some(&b'/') {
        bytes.push(b'/');
    }
    Ok(message(pli::RC_FOLDERDELETE, &bytes))
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
    let (name, rest) = file_parts(payload)?;
    if rest.len() > MAX_FILE_BYTES {
        return Err(FileError::TooLarge {
            len: rest.len(),
            max: MAX_FILE_BYTES,
        });
    }
    Ok((name, rest.to_vec()))
}

/// Name and bytes of an id 102 payload, with no size limit. A piece of a
/// large file is one of these.
fn file_parts(payload: &[u8]) -> Result<(String, &[u8]), FileError> {
    let mut i = 0;
    read_gint5(payload, &mut i)
        .ok_or_else(|| FileError::malformed("file time is shorter than a GINT5"))?;
    let name = read_gstring(payload, &mut i)
        .ok_or_else(|| FileError::malformed("file name length runs past the payload"))?;
    let mut rest = payload.get(i..).unwrap_or_default();
    if let Some(body) = rest.strip_suffix(b"\n") {
        rest = body;
    }
    Ok((text(&name), rest))
}

/// A large file being received: the size from id 84, if it came, and the
/// pieces so far.
#[derive(Default)]
struct LargeIn {
    size: Option<usize>,
    data: Vec<u8>,
}

impl LargeIn {
    /// Add one id 102 piece. More than the announced size, or more than
    /// [`MAX_LARGE_FILE_BYTES`], is an error.
    fn push(&mut self, payload: &[u8]) -> Result<(), FileError> {
        let (_name, piece) = file_parts(payload)?;
        let len = self.data.len() + piece.len();
        if len > MAX_LARGE_FILE_BYTES {
            return Err(FileError::TooLarge {
                len,
                max: MAX_LARGE_FILE_BYTES,
            });
        }
        if let Some(size) = self.size {
            if len > size {
                return Err(FileError::malformed(format!(
                    "large file pieces run to {len} bytes; the server said {size}"
                )));
            }
        }
        self.data.extend_from_slice(piece);
        Ok(())
    }

    /// The file, once id 69 has come. Fewer bytes than announced is an
    /// error.
    fn finish(self) -> Result<Vec<u8>, FileError> {
        match self.size {
            Some(size) if size != self.data.len() => Err(FileError::malformed(format!(
                "large file ended at {} bytes; the server said {size}",
                self.data.len()
            ))),
            _ => Ok(self.data),
        }
    }
}

/// The size in an id 84 payload.
fn parse_large_size(payload: &[u8]) -> Result<usize, FileError> {
    let mut i = 0;
    let size = read_gint5(payload, &mut i)
        .ok_or_else(|| FileError::malformed("large file size is shorter than a GINT5"))?;
    let size = size as usize;
    if size > MAX_LARGE_FILE_BYTES {
        return Err(FileError::TooLarge {
            len: size,
            max: MAX_LARGE_FILE_BYTES,
        });
    }
    Ok(size)
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
    /// `path`. A file the server sends as a large file (ids 68, 84, 102
    /// pieces, 69) is joined here. Other events stay queued.
    ///
    /// `timeout` covers the whole file, large or not.
    pub fn get_file(&mut self, path: &str, timeout: Duration) -> Result<Vec<u8>, FileError> {
        self.link.send(&[down_message(path)?])?;
        let deadline = Instant::now() + timeout;
        let mut large: Option<LargeIn> = None;
        self.wait_until(deadline, |ev| match ev {
            Event::Other(id, _) if id == plo::LARGEFILESTART => {
                large = Some(LargeIn::default());
                Step::Take
            }
            Event::Other(id, p) if id == plo::LARGEFILESIZE && large.is_some() => {
                match parse_large_size(&p) {
                    Ok(size) => {
                        if let Some(l) = large.as_mut() {
                            l.size = Some(size);
                        }
                        Step::Take
                    }
                    Err(e) => Step::Stop(e),
                }
            }
            Event::Other(id, p) if id == PLO_FILE => match large.as_mut() {
                Some(l) => match l.push(&p) {
                    Ok(()) => Step::Take,
                    Err(e) => Step::Stop(e),
                },
                None => match parse_file_body(&p) {
                    Ok((_name, bytes)) => Step::Done(bytes),
                    Err(e) => Step::Stop(e),
                },
            },
            Event::Other(id, _) if id == plo::LARGEFILEEND && large.is_some() => {
                match large.take().map(LargeIn::finish) {
                    Some(Ok(bytes)) => Step::Done(bytes),
                    Some(Err(e)) => Step::Stop(e),
                    None => Step::Take,
                }
            }
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
    /// A file up to [`MAX_FILE_BYTES`] goes in one block, and id 67
    /// beginning `Uploaded file ` is success. A bigger one goes as a large
    /// file (see [`large_upload_bodies`]), and id 67 beginning
    /// `Uploaded large file ` is success. Any other id 67 is a refusal. A
    /// file over [`MAX_LARGE_FILE_BYTES`] is refused and nothing is sent.
    pub fn put_file(
        &mut self,
        path: &str,
        data: &[u8],
        timeout: Duration,
    ) -> Result<(), FileError> {
        let ok = if data.len() <= MAX_FILE_BYTES {
            self.link.send_body(&upload_body(path, data)?)?;
            UPLOADED
        } else {
            for body in large_upload_bodies(path, data)? {
                self.link.send_body(&body)?;
            }
            UPLOADED_LARGE
        };
        let deadline = Instant::now() + timeout;
        self.wait_until(deadline, |ev| match ev {
            Event::Other(id, p) if id == plo::RC_FILEBROWSER_MESSAGE => {
                let msg = text(&p);
                if msg.starts_with(ok) {
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
        // 96: GSTRING folder, then the name as the rest of the line.
        assert_eq!(move_message("a.nw", "old/").unwrap(), b"\x80$old/a.nw");
        // 97: the name as the rest of the line.
        assert_eq!(delete_message("a.nw").unwrap(), b"\x81a.nw");
        // 98: two GSTRINGs.
        assert_eq!(rename_message("a.nw", "b.nw").unwrap(), b"\x82$a.nw$b.nw");
        // 160: the folder and a trailing slash as the rest of the line.
        assert_eq!(folder_delete_message("old/").unwrap(), b"\xc0old/");
        assert_eq!(
            folder_delete_message("world/labgone").unwrap(),
            b"\xc0world/labgone/"
        );
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
        // Id 50, GINT3(10), line feed, then id 93, GSTRING "a.nw", "hi\n",
        // and the block's own line feed, which the length counts. No GINT3
        // size in front of the file.
        assert_eq!(upload_body("a.nw", b"hi\n").unwrap(), b"R  *\n}$a.nwhi\n\n");
    }

    #[test]
    fn large_upload_is_start_pieces_and_end() {
        let data: Vec<u8> = (0..LARGE_CHUNK_BYTES + 5).map(|i| i as u8).collect();
        let bodies = large_upload_bodies("a.png", &data).unwrap();
        assert_eq!(bodies.len(), 4);
        // 155 and 156 are GCHARs 0xbb and 0xbc, the name, a line feed.
        assert_eq!(bodies[0], b"\xbba.png\n");
        assert_eq!(bodies[3], b"\xbca.png\n");
        // Each piece is an ordinary put block with the same name.
        assert_eq!(
            bodies[1],
            upload_body("a.png", &data[..LARGE_CHUNK_BYTES]).unwrap()
        );
        assert_eq!(
            bodies[2],
            upload_body("a.png", &data[LARGE_CHUNK_BYTES..]).unwrap()
        );
        // Every body still fits in one frame.
        assert!(bodies.iter().all(|b| b.len() < 65535));
        assert!(large_upload_bodies("../a.png", &data).is_err());
    }

    #[test]
    fn large_size_and_pieces_are_checked() {
        let mut p = Vec::new();
        push_gint5(&mut p, 5);
        assert_eq!(parse_large_size(&p).unwrap(), 5);
        assert!(parse_large_size(b"  ").is_err());
        let mut piece = Vec::new();
        push_gint5(&mut piece, 0);
        push_gstring(&mut piece, b"a");
        piece.extend_from_slice(b"abc\n");
        let mut l = LargeIn {
            size: Some(5),
            data: Vec::new(),
        };
        l.push(&piece).unwrap();
        assert!(matches!(l.push(&piece), Err(FileError::Malformed(_))));
        assert!(matches!(l.finish(), Err(FileError::Malformed(_))));
        // No size (a pre-2.3 server): the pieces are the file.
        let mut l = LargeIn::default();
        l.push(&piece).unwrap();
        l.push(&piece).unwrap();
        assert_eq!(l.finish().unwrap(), b"abcabc");
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
