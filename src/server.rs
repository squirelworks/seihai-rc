//! Stage 4 part B: server options, folder config and server flags.
//!
//! Ids are wire facts. Ids 51 to 54 meant account actions in RC 1.33; these
//! are the meanings from the 2002/05/02 RC on (`remotecontrol-lineage.md`
//! in the project files).
//!
//! Every layout here is **check**: no real RC was captured getting or
//! setting these yet. They are written the way the RC's other text
//! replies are (tokenized text, as for the comments in id 63), and the
//! flag list as a count and one `GSTRING` per flag. The command line reads
//! the value back after a set and says when it differs from what was sent,
//! so a wrong guess shows up on the first run against the fork.
//!
//! | Get | Reply | Set | Payload (**check**) |
//! |---|---|---|---|
//! | 51 | 76 | 52 | options text, tokenized, one item per line |
//! | 53 | 77 | 54 | folder config text, tokenized, one item per line |
//! | 68 | 61 | 69 | `GSHORT` count, then each flag (`name=value`) as a `GSTRING` |

use crate::wire::{
    gdetokenize, gtokenize, latin1_bytes, message, push_gshort, push_gstring, read_gstring, text,
};
use seihai_core::protocol::read_gshort;

/// Client to server ids.
pub mod pli {
    pub const RC_SERVEROPTIONSGET: u8 = 51;
    pub const RC_SERVEROPTIONSSET: u8 = 52;
    pub const RC_FOLDERCONFIGGET: u8 = 53;
    pub const RC_FOLDERCONFIGSET: u8 = 54;
    pub const RC_SERVERFLAGSGET: u8 = 68;
    pub const RC_SERVERFLAGSSET: u8 = 69;
}

/// Server to client ids.
pub mod plo {
    pub const RC_SERVERFLAGSGET: u8 = 61;
    pub const RC_SERVEROPTIONSGET: u8 = 76;
    pub const RC_FOLDERCONFIGGET: u8 = 77;
}

/// The longest flag a `GSTRING` can carry.
pub const FLAG_MAX: usize = 223;

/// Which text the server keeps: options or folder config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerText {
    Options,
    Folders,
}

impl ServerText {
    /// Get id, reply id, set id.
    pub fn ids(self) -> (u8, u8, u8) {
        match self {
            ServerText::Options => (
                pli::RC_SERVEROPTIONSGET,
                plo::RC_SERVEROPTIONSGET,
                pli::RC_SERVEROPTIONSSET,
            ),
            ServerText::Folders => (
                pli::RC_FOLDERCONFIGGET,
                plo::RC_FOLDERCONFIGGET,
                pli::RC_FOLDERCONFIGSET,
            ),
        }
    }

    /// The get message: the id, no payload.
    pub fn get_message(self) -> Vec<u8> {
        message(self.ids().0, b"")
    }

    /// The set message: the text tokenized, one item per line. `\r\n`
    /// counts as one line break. **check**
    pub fn set_message(self, lines: &str) -> Vec<u8> {
        let tokenized = gtokenize(&lines.replace("\r\n", "\n"));
        message(self.ids().2, &latin1_bytes(&tokenized))
    }
}

/// Read a 76 or 77 reply: tokenized text, one line per item. **check**
pub fn parse_text(payload: &[u8]) -> Vec<String> {
    gdetokenize(&text(payload))
}

/// The flags get message.
pub fn flags_get() -> Vec<u8> {
    message(pli::RC_SERVERFLAGSGET, b"")
}

/// The flags set message: `GSHORT` count, then one `GSTRING` per flag.
/// Empty lines are dropped. A flag over [`FLAG_MAX`] bytes or with a line
/// feed is refused. **check**
pub fn flags_set(flags: &[String]) -> Result<Vec<u8>, String> {
    let flags: Vec<&String> = flags.iter().filter(|f| !f.trim().is_empty()).collect();
    let count = u16::try_from(flags.len()).map_err(|_| "too many flags".to_string())?;
    let mut p = Vec::new();
    push_gshort(&mut p, count);
    for f in flags {
        let b = latin1_bytes(f);
        if b.len() > FLAG_MAX || b.contains(&b'\n') {
            return Err(format!(
                "flag is longer than {FLAG_MAX} bytes or has a line break: {f}"
            ));
        }
        push_gstring(&mut p, &b);
    }
    Ok(message(pli::RC_SERVERFLAGSSET, &p))
}

/// Read a 61 reply: `GSHORT` count, then that many `GSTRING`s. **check**
pub fn parse_flags(payload: &[u8]) -> Option<Vec<String>> {
    let n = read_gshort(payload)?;
    let mut i = 2;
    let mut out = Vec::with_capacity(usize::from(n));
    for _ in 0..n {
        out.push(text(&read_gstring(payload, &mut i)?));
    }
    Some(out)
}

/// Lines as the command line compares them: trailing spaces and `\r`
/// dropped, empty lines left out.
pub fn normalized(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|l| l.trim_end().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::gchar;

    #[test]
    fn gets_are_bare_ids() {
        assert_eq!(ServerText::Options.get_message(), [gchar(51)]);
        assert_eq!(ServerText::Folders.get_message(), [gchar(53)]);
        assert_eq!(flags_get(), [gchar(68)]);
    }

    #[test]
    fn text_set_is_tokenized_and_reads_back() {
        let m = ServerText::Options.set_message("name=Lab\r\nstaff=labrc,admin");
        assert_eq!(m[0], gchar(52));
        assert_eq!(&m[1..], b"\"name=Lab\",\"staff=labrc,admin\"");
        assert_eq!(parse_text(&m[1..]), ["name=Lab", "staff=labrc,admin"]);
        let f = ServerText::Folders.set_message("file rw levels/*");
        assert_eq!(f[0], gchar(54));
    }

    #[test]
    fn flags_are_a_count_then_gstrings() {
        let flags = vec!["a=1".to_string(), "".to_string(), "b".to_string()];
        let m = flags_set(&flags).unwrap();
        // 69, GSHORT 2, GSTRING "a=1", GSTRING "b".
        assert_eq!(m, b"e \"#a=1!b");
        assert_eq!(parse_flags(&m[1..]).unwrap(), ["a=1", "b"]);
        assert!(flags_set(&["x".repeat(FLAG_MAX + 1)]).is_err());
        // A count that runs past the payload.
        assert!(parse_flags(&[gchar(0), gchar(2), gchar(1), b'a']).is_none());
    }

    #[test]
    fn normalized_drops_blank_lines_and_trailing_spaces() {
        let l = vec!["a ".to_string(), "".to_string(), "b\r".to_string()];
        assert_eq!(normalized(&l), ["a", "b"]);
    }
}
