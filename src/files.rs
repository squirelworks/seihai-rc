//! File link on an RC session: list a directory, fetch a file, send a file.
//!
//! The ids are the file-browser ids named in `docs/NEXT.md`. This repo does
//! not describe the payloads. What follows is the smallest framing that
//! fits the codec this crate already uses (`GSTRING`, `GSHORT`, `GINT3`,
//! and the raw block `split_messages` keeps intact). A level file contains
//! line feeds, and [`crate::link::Link::send`] refuses a line feed inside an
//! ordinary message, so the file bytes travel in a raw block.
//!
//! # Payload fields
//!
//! A loopback run against the gserver-v2 fork (2026-11-01,
//! `docs/lab/2026-11-01-file-widths.md`) did not match this layout. The
//! client still sends it. Do not treat it as what that fork sends.
//!
//! - List request, id [`pli::RC_FILEBROWSER_CD`] (90): one `GSTRING` path.
//!   `""` means the world root. Whether id 89 `START` has to come first is
//!   unchecked; this client does not send it.
//! - Name list, id [`plo::RC_FILEBROWSER_DIRLIST`] (65): a `GSHORT` count,
//!   then that many `GSTRING` names. No size and no directory flag. The
//!   count being a `GSHORT` (not a `GCHAR`) is unchecked.
//! - Get request, id [`pli::RC_FILEBROWSER_DOWN`] (92): one `GSTRING` path.
//! - File bytes, inbound: a raw block of id [`PLO_RAWDATA`] (100), then a
//!   message of id [`plo::RC_FILEBROWSER_DIR`] (66) whose payload is a
//!   `GSTRING` name, a `GINT3` size, and that many bytes. The id 66, the
//!   name field and the `GINT3` are all unchecked. Id 100 is used because
//!   `split_messages` already treats it as a raw block.
//! - File bytes, outbound: the same name, `GINT3` and bytes, in a message
//!   of id [`pli::RC_FILEBROWSER_UP`] (93), wrapped in a raw block of id
//!   [`PLI_RAWDATA`] (50). Id 50 is the outbound raw id named in the
//!   NPC-Control section of `docs/PROTOCOL.md`. Copying the inbound `GINT3`
//!   header onto it is unchecked, and `split_messages` does not treat id 50
//!   as raw.
//! - Refusal, id [`plo::RC_FILEBROWSER_MESSAGE`] (67): the text.
//!
//! Not sent, layouts unknown: 89, 91, 96, 97, 98, 155, 156, 160. A file
//! over [`MAX_FILE_BYTES`] is refused. One frame length is a `u16`, and the
//! large-file ids are how a bigger file would travel once a lab run shows
//! their layout.

use std::io;
use std::time::{Duration, Instant};

use seihai_core::protocol::{read_gint3, read_gshort};

use crate::rc::{Event, Session};
use crate::wire::{gchar, latin1_bytes, message, push_gstring, read_gstring, text};

/// Client to server. Raw block: this id, a `GINT3` byte count, a line feed,
/// then that many bytes. Named in `docs/PROTOCOL.md` as the outbound raw id.
pub const PLI_RAWDATA: u8 = 50;

/// Server to client. The same header shape. `split_messages` keeps the
/// following bytes together, line feeds included.
pub const PLO_RAWDATA: u8 = 100;

/// Largest file [`Session::put_file`] sends and [`Session::get_file`] accepts.
///
/// An incompressible file plus the raw-block header has to fit in one frame,
/// and a frame length is a `u16` (65535). 60 KiB leaves room for the header
/// and for zlib growing a file that does not compress.
pub const MAX_FILE_BYTES: usize = 60 * 1024;

/// Client to server file-browser ids. Only CD, DOWN and UP are sent.
pub mod pli {
    /// Not sent. Payload unchecked.
    pub const RC_FILEBROWSER_START: u8 = 89;
    /// List the directory in the `GSTRING` path.
    pub const RC_FILEBROWSER_CD: u8 = 90;
    /// Not sent. Payload unchecked.
    pub const RC_FILEBROWSER_END: u8 = 91;
    /// Fetch the file in the `GSTRING` path.
    pub const RC_FILEBROWSER_DOWN: u8 = 92;
    /// Send a file. The message sits inside [`super::PLI_RAWDATA`].
    pub const RC_FILEBROWSER_UP: u8 = 93;
    /// Not sent. Payload unchecked.
    pub const RC_FILEBROWSER_MOVE: u8 = 96;
    /// Not sent. Payload unchecked.
    pub const RC_FILEBROWSER_DELETE: u8 = 97;
    /// Not sent. Payload unchecked.
    pub const RC_FILEBROWSER_RENAME: u8 = 98;
    /// Not sent. Payload unchecked. For a file that does not fit in one frame.
    pub const RC_LARGEFILESTART: u8 = 155;
    /// Not sent. Payload unchecked.
    pub const RC_LARGEFILEEND: u8 = 156;
    /// Not sent. Payload unchecked.
    pub const RC_FOLDERDELETE: u8 = 160;
}

/// Server to client file-browser ids.
pub mod plo {
    /// Directory names. `GSHORT` count, then that many `GSTRING` names.
    pub const RC_FILEBROWSER_DIRLIST: u8 = 65;
    /// File body, inside [`super::PLO_RAWDATA`]. Payload unchecked.
    pub const RC_FILEBROWSER_DIR: u8 = 66;
    /// A refusal. The payload is text.
    pub const RC_FILEBROWSER_MESSAGE: u8 = 67;
}

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
    /// A length ran past the payload, or a size did not match the bytes.
    Malformed(String),
    /// `len` bytes is over [`MAX_FILE_BYTES`].
    TooLarge {
        len: usize,
        max: usize,
    },
    /// The path is absolute, walks up with `..`, or does not fit a `GSTRING`.
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

/// `GINT3`: three GCHARs, high 7 bits first. Matches `read_gint3`.
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

/// Relative `/` path as Latin-1, or an error. `allow_empty` is the world root.
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
    for seg in path.split('/') {
        if seg.is_empty() || seg == ".." || seg.contains('\\') || seg.contains(':') {
            return Err(FileError::InvalidPath(format!(
                "path leaves the world or is not /-separated: {path}"
            )));
        }
    }
    let bytes = latin1_bytes(path);
    if bytes.len() > 223 || bytes.contains(&b'\n') || bytes.contains(&0u8) {
        return Err(FileError::InvalidPath(
            "path does not fit in one GSTRING".into(),
        ));
    }
    Ok(bytes)
}

/// List request. The path is one `GSTRING` (lab-unconfirmed; see the module).
pub fn cd_message(dir: &str) -> Result<Vec<u8>, FileError> {
    let path = path_bytes(dir, true)?;
    let mut payload = Vec::new();
    push_gstring(&mut payload, &path);
    Ok(message(pli::RC_FILEBROWSER_CD, &payload))
}

/// Get request. The path is one `GSTRING` (lab-unconfirmed).
pub fn down_message(path: &str) -> Result<Vec<u8>, FileError> {
    let path = path_bytes(path, false)?;
    let mut payload = Vec::new();
    push_gstring(&mut payload, &path);
    Ok(message(pli::RC_FILEBROWSER_DOWN, &payload))
}

/// Put body, ready for [`crate::link::Link::send_body`].
///
/// The raw header id and the `GINT3` size are lab-unconfirmed. A file over
/// [`MAX_FILE_BYTES`] is refused and nothing is built for it.
pub fn upload_body(path: &str, data: &[u8]) -> Result<Vec<u8>, FileError> {
    if data.len() > MAX_FILE_BYTES {
        return Err(FileError::TooLarge {
            len: data.len(),
            max: MAX_FILE_BYTES,
        });
    }
    let path = path_bytes(path, false)?;
    let mut inner = Vec::with_capacity(5 + path.len() + data.len());
    inner.push(gchar(pli::RC_FILEBROWSER_UP));
    push_gstring(&mut inner, &path);
    push_gint3(&mut inner, data.len());
    inner.extend_from_slice(data);
    Ok(raw_block(PLI_RAWDATA, &inner))
}

/// Names from a directory-list payload.
///
/// The `GSHORT` count and the `GSTRING` names are lab-unconfirmed. A count
/// or a name length that runs past the payload is an error.
pub fn parse_dir_list(payload: &[u8]) -> Result<Vec<String>, FileError> {
    let count = read_gshort(payload)
        .ok_or_else(|| FileError::malformed("directory list is shorter than a count"))?
        as usize;
    let mut i = 2;
    let mut names = Vec::new();
    for n in 0..count {
        let raw = read_gstring(payload, &mut i).ok_or_else(|| {
            FileError::malformed(format!("name {n} length runs past the directory list"))
        })?;
        names.push(text(&raw));
    }
    if i != payload.len() {
        return Err(FileError::malformed("bytes left after the directory names"));
    }
    Ok(names)
}

/// `(name, bytes)` from a file-message payload.
///
/// The name and the `GINT3` size are lab-unconfirmed. A size over
/// [`MAX_FILE_BYTES`] is refused without copying those bytes out. A size
/// that does not match the bytes that follow is an error.
pub fn parse_file_body(payload: &[u8]) -> Result<(String, Vec<u8>), FileError> {
    let mut i = 0;
    let name = read_gstring(payload, &mut i)
        .ok_or_else(|| FileError::malformed("file name length runs past the payload"))?;
    let Some(size_at) = payload.get(i..i + 3) else {
        return Err(FileError::malformed("file size is shorter than a GINT3"));
    };
    let Some(size) = read_gint3(size_at) else {
        return Err(FileError::malformed("file size is shorter than a GINT3"));
    };
    i += 3;
    if size > MAX_FILE_BYTES {
        return Err(FileError::TooLarge {
            len: size,
            max: MAX_FILE_BYTES,
        });
    }
    let rest = payload.get(i..).unwrap_or_default();
    if rest.len() != size {
        return Err(FileError::malformed(format!(
            "file length {size} but {} bytes follow the size",
            rest.len()
        )));
    }
    Ok((text(&name), rest.to_vec()))
}

enum Step<T> {
    Done(T),
    Keep(Event),
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

    /// The names in `dir`, in the order the server sends them.
    ///
    /// `""` is the world root. Events that are not the list (the login
    /// signature, chat, players) stay queued.
    pub fn list_dir(&mut self, dir: &str, timeout: Duration) -> Result<Vec<String>, FileError> {
        self.link.send(&[cd_message(dir)?])?;
        let deadline = Instant::now() + timeout;
        self.wait_until(deadline, |ev| match ev {
            Event::Other(id, p) if id == plo::RC_FILEBROWSER_DIRLIST => match parse_dir_list(&p) {
                Ok(names) => Step::Done(names),
                Err(e) => Step::Stop(e),
            },
            Event::Other(id, p) if id == plo::RC_FILEBROWSER_MESSAGE => {
                Step::Stop(FileError::Refused(text(&p)))
            }
            Event::Disconnect(why) => Step::Stop(FileError::Refused(why)),
            other => Step::Keep(other),
        })
    }

    /// The bytes of `path`, whatever the server sent for that get.
    ///
    /// The name carried beside the bytes is not compared with `path` (that
    /// field is lab-unconfirmed). Other events stay queued.
    pub fn get_file(&mut self, path: &str, timeout: Duration) -> Result<Vec<u8>, FileError> {
        self.link.send(&[down_message(path)?])?;
        let deadline = Instant::now() + timeout;
        self.wait_until(deadline, |ev| match ev {
            Event::Other(id, p) if id == plo::RC_FILEBROWSER_DIR => match parse_file_body(&p) {
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

    /// Send `data` as `path`. Returns once the frame is written.
    ///
    /// A file over [`MAX_FILE_BYTES`] is refused and nothing is sent. The
    /// server's copy is whatever it recorded from this frame; this call
    /// does not wait for an ack.
    pub fn put_file(&mut self, path: &str, data: &[u8]) -> Result<(), FileError> {
        self.link.send_body(&upload_body(path, data)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use seihai_core::protocol::split_messages;

    use super::*;

    #[test]
    fn cd_and_down_bytes() {
        assert_eq!(cd_message("levels").unwrap(), b"z&levels");
        assert_eq!(cd_message("").unwrap(), b"z ");
        assert_eq!(down_message("a.nw").unwrap(), b"|$a.nw");
    }

    #[test]
    fn upload_bytes_keep_the_line_feed() {
        // Id 50, GINT3(12), line feed, then id 93, GSTRING "a.nw",
        // GINT3(3), "hi\n".
        assert_eq!(
            upload_body("a.nw", b"hi\n").unwrap(),
            b"R  ,\n}$a.nw  #hi\n"
        );
    }

    #[test]
    fn inbound_file_with_a_line_feed_survives_split_messages() {
        let data = b"GLEVNW01\n\xff";
        let mut inner = vec![gchar(plo::RC_FILEBROWSER_DIR)];
        push_gstring(&mut inner, b"a.nw");
        push_gint3(&mut inner, data.len());
        inner.extend_from_slice(data);
        let msgs = split_messages(&raw_block(PLO_RAWDATA, &inner));
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].id, plo::RC_FILEBROWSER_DIR);
        let (name, bytes) = parse_file_body(&msgs[0].payload).unwrap();
        assert_eq!(name, "a.nw");
        assert_eq!(bytes, data);
    }

    #[test]
    fn dir_list_round_trip_and_empty() {
        let mut p = vec![gchar(0), gchar(2)];
        push_gstring(&mut p, b"a.nw");
        push_gstring(&mut p, &[0xE9]);
        assert_eq!(parse_dir_list(&p).unwrap(), ["a.nw", "é"]);
        assert!(parse_dir_list(&[gchar(0), gchar(0)]).unwrap().is_empty());
    }

    #[test]
    fn malformed_lengths_are_errors() {
        assert!(parse_dir_list(b"").is_err());
        assert!(parse_dir_list(&[0x20]).is_err());
        // Count 1, length byte 0x00 wraps to 224, and no name bytes follow.
        assert!(matches!(
            parse_dir_list(&[0x20, 0x21, 0x00]),
            Err(FileError::Malformed(_))
        ));
        // Count 2, one short name.
        let mut p = vec![gchar(0), gchar(2)];
        push_gstring(&mut p, b"only");
        assert!(matches!(parse_dir_list(&p), Err(FileError::Malformed(_))));
        assert!(parse_file_body(b"").is_err());
        // Name "a", then only two of the three size bytes.
        assert!(matches!(
            parse_file_body(&[gchar(1), b'a', 0x20, 0x20]),
            Err(FileError::Malformed(_))
        ));
        // Size 5, two bytes follow.
        let mut p = vec![gchar(1), b'a'];
        push_gint3(&mut p, 5);
        p.extend_from_slice(b"xy");
        assert!(matches!(parse_file_body(&p), Err(FileError::Malformed(_))));
    }

    #[test]
    fn too_large_file_is_refused() {
        let big = vec![0u8; MAX_FILE_BYTES + 1];
        assert!(matches!(
            upload_body("a.nw", &big),
            Err(FileError::TooLarge { len, max }) if len == big.len() && max == MAX_FILE_BYTES
        ));
        let mut p = vec![gchar(1), b'a'];
        push_gint3(&mut p, MAX_FILE_BYTES + 1);
        assert!(matches!(
            parse_file_body(&p),
            Err(FileError::TooLarge { len, .. }) if len == MAX_FILE_BYTES + 1
        ));
        // The size is past the limit and the bytes are not even there.
        // Still a size error, and it does not try to allocate `len`.
    }

    #[test]
    fn paths_outside_the_world_are_refused() {
        for bad in ["../x", "/outside/a.nw", "a/../../b", "a\\b", "C:/a.nw", ""] {
            assert!(down_message(bad).is_err(), "{bad}");
            assert!(upload_body(bad, b"z").is_err(), "{bad}");
        }
        assert!(cd_message("../x").is_err());
        assert!(cd_message("").is_ok());
    }

    #[test]
    fn empty_file_round_trip() {
        let mut p = Vec::new();
        push_gstring(&mut p, b"a.nw");
        push_gint3(&mut p, 0);
        let (name, bytes) = parse_file_body(&p).unwrap();
        assert_eq!(name, "a.nw");
        assert!(bytes.is_empty());
    }
}
