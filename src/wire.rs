//! Small encoders for the Graal wire types the RC sends.
//!
//! Every number on the wire is a "G" type: each byte carries 7 bits plus
//! `0x20`, so no byte is a line feed. A message is `GCHAR id` + payload +
//! `\n`. Decoding helpers come from `seihai-core`.

/// One GCHAR: `n + 0x20`.
pub fn gchar(n: u8) -> u8 {
    n.wrapping_add(0x20)
}

/// Append a GSHORT: two GCHARs, high 7 bits first.
pub fn push_gshort(out: &mut Vec<u8>, n: u16) {
    out.push(gchar(((n >> 7) & 0x7F) as u8));
    out.push(gchar((n & 0x7F) as u8));
}

/// Append a GINT3: three GCHARs, high 7 bits first. Twenty-one bits.
pub fn push_gint3(out: &mut Vec<u8>, n: u32) {
    out.push(gchar(((n >> 14) & 0x7F) as u8));
    out.push(gchar(((n >> 7) & 0x7F) as u8));
    out.push(gchar((n & 0x7F) as u8));
}

/// Append a GINT5: five GCHARs, high 7 bits first. Thirty-five bits; a
/// `u32` always fits.
pub fn push_gint5(out: &mut Vec<u8>, n: u32) {
    let n = u64::from(n);
    for shift in [28, 21, 14, 7, 0] {
        out.push(gchar(((n >> shift) & 0x7F) as u8));
    }
}

/// Read a GINT5 at `*i`, moving `*i` past it. A value over 32 bits is
/// refused.
pub fn read_gint5(data: &[u8], i: &mut usize) -> Option<u32> {
    let bytes = data.get(*i..*i + 5)?;
    let mut n: u64 = 0;
    for &b in bytes {
        n = (n << 7) | u64::from(b.wrapping_sub(0x20) & 0x7F);
    }
    *i += 5;
    u32::try_from(n).ok()
}

/// Append a GSTRING: GCHAR length, then the bytes. Text longer than 223
/// bytes is cut, as a GCHAR cannot say more.
pub fn push_gstring(out: &mut Vec<u8>, s: &[u8]) {
    let s = &s[..s.len().min(223)];
    out.push(gchar(s.len() as u8));
    out.extend_from_slice(s);
}

/// Read a GSTRING at `*i`, moving `*i` past it.
pub fn read_gstring(data: &[u8], i: &mut usize) -> Option<Vec<u8>> {
    let n = data.get(*i)?.wrapping_sub(0x20) as usize;
    let s = data.get(*i + 1..*i + 1 + n)?.to_vec();
    *i += 1 + n;
    Some(s)
}

/// A message: `GCHAR id` + payload. The link adds the line feed.
pub fn message(id: u8, payload: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(1 + payload.len());
    m.push(gchar(id));
    m.extend_from_slice(payload);
    m
}

/// Text as the server stores it: Latin-1 bytes to a Rust string.
pub fn text(b: &[u8]) -> String {
    seihai_core::protocol::latin1(b)
}

/// A Rust string to Latin-1 bytes; characters outside Latin-1 become `?`.
pub fn latin1_bytes(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
        .collect()
}

/// Graal's tokenized text, as the gserver and the RC write it: one item
/// per line, joined with commas. The encoder is `seihai-core`'s
/// `gtokenize` (every item quoted, `"` doubled); see [`gtokenize`].
pub fn gtokenize(lines: &str) -> String {
    seihai_core::net::gtokenize(lines)
}

/// Graal's tokenized text the way the real RCs write it (lab 2026-11-13):
/// one item per line, quoted only when it starts with a quote or holds a
/// byte outside 33..126, a comma or a slash; inside quotes `\` and `"` are
/// doubled. An empty line is an empty item. Unlike [`gtokenize`], a plain
/// word stays bare: the 2004 RC sent `}` and `labnpcflag=1` unquoted.
///
/// This is the GServer-v2 form. Use [`rc_tokenize_for`] when the server is
/// known: seihai-gserver keeps backslashes as sent, so doubling them there
/// would double every backslash on each save (#56).
pub fn rc_tokenize(lines: &[&str]) -> String {
    rc_tokenize_for(ServerTextProfile::GserverV2Seihai, lines)
}

/// [`rc_tokenize`] for `profile`: the encoder [`gdetokenize_for`] reverses.
/// Backslashes inside quotes are doubled only where the profile's server
/// reads them doubled.
pub fn rc_tokenize_for(profile: ServerTextProfile, lines: &[&str]) -> String {
    let double = profile.doubles_quoted_backslashes();
    lines
        .iter()
        .map(|line| {
            let complex = line.starts_with('"')
                || line
                    .chars()
                    .any(|c| (c as u32) < 33 || (c as u32) > 126 || c == ',' || c == '/');
            if complex {
                let line = if double {
                    line.replace('\\', "\\\\")
                } else {
                    (*line).to_string()
                };
                format!("\"{}\"", line.replace('"', "\"\""))
            } else {
                (*line).to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// How a connected server encodes tokenized RC text.
///
/// Same idea as client version profiles: identical wire bytes can mean
/// different logical text depending on the producer. Quirks live in this
/// table. Unknown servers use [`Self::SeihaiGserver`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ServerTextProfile {
    /// seihai-gserver: backslashes in quoted fields stay as sent. Default
    /// for an unknown server.
    #[default]
    SeihaiGserver,
    /// GServer-v2 `seihai` branch: backslashes are doubled inside quoted
    /// fields on the wire, so a GET decoder must undouble them.
    GserverV2Seihai,
    /// GServer-v2 upstream beta4. Pinned from its source at `bc54973`
    /// (2026-11-17): NPC, class and flag text goes out through its CSV
    /// writer, which quotes a line holding `"`, `,`, `\` or a byte outside
    /// 32..126 and doubles both `"` and `\` inside the quotes. So it reads
    /// like [`Self::GserverV2Seihai`] for backslashes.
    GserverV2Beta4,
}

impl ServerTextProfile {
    /// CLI / config names.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SeihaiGserver => "seihai-gserver",
            Self::GserverV2Seihai => "gserver-v2-seihai",
            Self::GserverV2Beta4 => "gserver-v2-beta4",
        }
    }

    /// Parse a CLI name. Accepts the `as_str` forms and a few short aliases.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "seihai-gserver" | "seihai" | "gserver" => Some(Self::SeihaiGserver),
            "gserver-v2-seihai" | "gserver-v2" | "fork" => Some(Self::GserverV2Seihai),
            "gserver-v2-beta4" | "beta4" => Some(Self::GserverV2Beta4),
            _ => None,
        }
    }

    /// Whether the server doubles `\` inside quoted fields, both in what
    /// it sends and in what it expects to read.
    pub fn doubles_quoted_backslashes(self) -> bool {
        match self {
            Self::SeihaiGserver => false,
            Self::GserverV2Seihai | Self::GserverV2Beta4 => true,
        }
    }

    /// Whether quoted fields undouble `\\` to `\`.
    pub fn undouble_quoted_backslashes(self) -> bool {
        self.doubles_quoted_backslashes()
    }

    /// The profile a server names in its `Server: ` line (id 74, sent in
    /// the NC login burst): seihai-gserver sends `Server: seihai-gserver
    /// VERSION`, the GServer-v2 `seihai` branch `Server: GServer-v2 (Seihai)
    /// VERSION`. Any other `GServer-v2` or `GS2Emu` name is taken as
    /// upstream beta4. `None` for any other line.
    pub fn from_identity(line: &str) -> Option<Self> {
        let name = line
            .trim()
            .strip_prefix("Server:")?
            .trim()
            .to_ascii_lowercase();
        if name.starts_with("seihai-gserver") {
            Some(Self::SeihaiGserver)
        } else if name.starts_with("gserver-v2") || name.starts_with("gs2emu") {
            if name.contains("seihai") {
                Some(Self::GserverV2Seihai)
            } else {
                Some(Self::GserverV2Beta4)
            }
        } else {
            None
        }
    }
}

impl std::fmt::Display for ServerTextProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Read tokenized text back into lines with the seihai-gserver profile
/// (literal backslashes). Prefer [`gdetokenize_for`] when the peer's
/// profile is known.
pub fn gdetokenize(text: &str) -> Vec<String> {
    gdetokenize_for(ServerTextProfile::SeihaiGserver, text)
}

/// Read tokenized text back into lines for `profile`.
///
/// A quoted item runs to its closing quote (`""` inside is one `"`). With
/// [`ServerTextProfile::GserverV2Seihai`], `\\` inside quotes becomes one
/// `\`; other profiles keep backslashes literal. An unquoted item runs to
/// the next comma. A final comma adds an empty field. Spaces around an
/// item are dropped. Empty text gives no lines.
pub fn gdetokenize_for(profile: ServerTextProfile, text: &str) -> Vec<String> {
    let undouble = profile.undouble_quoted_backslashes();
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    loop {
        while chars.peek() == Some(&' ') {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }
        let mut item = String::new();
        if chars.peek() == Some(&'"') {
            chars.next();
            while let Some(c) = chars.next() {
                if c == '"' {
                    if chars.peek() == Some(&'"') {
                        chars.next();
                        item.push('"');
                    } else {
                        break;
                    }
                } else if c == '\\' && undouble && chars.peek() == Some(&'\\') {
                    chars.next();
                    item.push('\\');
                } else {
                    item.push(c);
                }
            }
            // Anything between the closing quote and the comma is dropped.
            for c in chars.by_ref() {
                if c == ',' {
                    break;
                }
            }
        } else {
            for c in chars.by_ref() {
                if c == ',' {
                    break;
                }
                item.push(c);
            }
            item = item.trim_end().to_string();
        }
        out.push(item);
        if text.ends_with(',') && chars.peek().is_none() {
            out.push(String::new());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rc_text_preserves_escaped_content_and_empty_fields() {
        let lines = [r#"echo("hello", 1);"#, "", "ordinary", "", ""];
        assert_eq!(gdetokenize(&rc_tokenize(&lines)), lines);
        assert_eq!(gdetokenize("a,"), ["a", ""]);
        assert_eq!(gdetokenize(",,"), ["", "", ""]);
        assert_eq!(gdetokenize(r#""a,""b"",c""#), ["a,\"b\",c"]);
        assert_eq!(gdetokenize(r#"C:\tmp"#), [r#"C:\tmp"#]);
        assert!(gdetokenize("").is_empty());
    }

    /// One fixture per server text profile: same logical script, the wire
    /// each producer emits for a quoted line with a backslash, and the
    /// decode that profile must return.
    #[test]
    fn server_text_profile_fixtures_round_trip_quoted_backslash() {
        // Logical script text an editor should show for one backslash.
        let logical = r#"echo("C:\tmp", 1);"#;
        // seihai-gserver quotes every field and doubles `"` only, so one
        // backslash on the wire means one backslash in the script.
        let seihai_wire = r#""echo(""C:\tmp"", 1);""#;
        assert_eq!(
            gdetokenize_for(ServerTextProfile::SeihaiGserver, seihai_wire),
            [logical]
        );
        // GServer-v2 seihai doubles backslashes inside quotes. The same
        // logical text therefore arrives with `\\` on the wire.
        let fork_wire = r#""echo(""C:\\tmp"", 1);""#;
        assert_eq!(
            gdetokenize_for(ServerTextProfile::GserverV2Seihai, fork_wire),
            [logical]
        );
        // Without that profile the same fork bytes stay doubled (ambiguous
        // without a producer contract).
        assert_eq!(
            gdetokenize_for(ServerTextProfile::SeihaiGserver, fork_wire),
            [r#"echo("C:\\tmp", 1);"#]
        );
        // beta4 doubles backslashes inside quotes like the fork.
        assert_eq!(
            gdetokenize_for(ServerTextProfile::GserverV2Beta4, fork_wire),
            [logical]
        );
        // Unquoted bare path is never undoubled.
        assert_eq!(
            gdetokenize_for(ServerTextProfile::GserverV2Seihai, r#"C:\\tmp"#),
            [r#"C:\\tmp"#]
        );
        assert_eq!(
            ServerTextProfile::parse("fork"),
            Some(ServerTextProfile::GserverV2Seihai)
        );
        assert_eq!(ServerTextProfile::default().as_str(), "seihai-gserver");
    }

    /// Each profile's encoder is the one its decoder reverses, so a save and
    /// a read back give the same script (#56). For seihai-gserver the old
    /// doubled form grew one backslash per save.
    #[test]
    fn server_text_profiles_encode_what_they_decode() {
        let lines = [r#"echo("C:\tmp", 1);"#, "}", r#"x = "a\\b";"#, ""];
        for p in [
            ServerTextProfile::SeihaiGserver,
            ServerTextProfile::GserverV2Seihai,
            ServerTextProfile::GserverV2Beta4,
        ] {
            assert_eq!(
                gdetokenize_for(p, &rc_tokenize_for(p, &lines)),
                lines,
                "{p}"
            );
        }
        assert_eq!(
            rc_tokenize_for(ServerTextProfile::SeihaiGserver, &[r#"a\b c"#]),
            r#""a\b c""#
        );
        assert_eq!(
            rc_tokenize_for(ServerTextProfile::GserverV2Seihai, &[r#"a\b c"#]),
            r#""a\\b c""#
        );
        assert_eq!(rc_tokenize(&[r#"a\b c"#]), r#""a\\b c""#);
    }

    #[test]
    fn server_text_profile_from_identity() {
        use ServerTextProfile as P;
        assert_eq!(
            P::from_identity("Server: seihai-gserver 0.9.0"),
            Some(P::SeihaiGserver)
        );
        assert_eq!(
            P::from_identity("Server: GServer-v2 (Seihai) 4.0.0-beta"),
            Some(P::GserverV2Seihai)
        );
        assert_eq!(
            P::from_identity("Server: GS2Emu 4.0.0-beta"),
            Some(P::GserverV2Beta4)
        );
        assert_eq!(P::from_identity("New NC: labnc"), None);
        assert_eq!(P::from_identity("Server: something else"), None);
    }

    #[test]
    fn gint5_matches_the_capture() {
        // Rights 513 in the 2026-11-04 capture: `   $!`.
        let mut b = Vec::new();
        push_gint5(&mut b, 513);
        assert_eq!(b, b"   $!");
        let mut i = 0;
        assert_eq!(read_gint5(&b, &mut i), Some(513));
        assert_eq!(i, 5);
        let mut big = Vec::new();
        push_gint5(&mut big, u32::MAX);
        let mut i = 0;
        assert_eq!(read_gint5(&big, &mut i), Some(u32::MAX));
        assert_eq!(read_gint5(b"    ", &mut 0), None);
    }

    #[test]
    fn tokenized_text_round_trips() {
        // The comments the real RC sent in the 2026-11-04 capture.
        let wire = "\"lab comment one\",\"line two\"";
        assert_eq!(gtokenize("lab comment one\nline two"), wire);
        assert_eq!(gdetokenize(wire), ["lab comment one", "line two"]);
        assert_eq!(
            gdetokenize(&gtokenize("say \"hi\", ok")),
            ["say \"hi\", ok"]
        );
        // Unquoted items, as the fork returns folder rights.
        assert_eq!(gdetokenize("rw levels/*"), ["rw levels/*"]);
        assert_eq!(gdetokenize("a,b ,\"\""), ["a", "b", ""]);
        assert!(gdetokenize("").is_empty());
    }

    #[test]
    fn gstring_round_trip() {
        let mut v = Vec::new();
        push_gstring(&mut v, b"admin");
        assert_eq!(v, b"%admin");
        let mut i = 0;
        assert_eq!(read_gstring(&v, &mut i).unwrap(), b"admin");
        assert_eq!(i, 6);
    }

    #[test]
    fn latin1_keeps_high_bytes() {
        assert_eq!(latin1_bytes("é"), vec![0xE9]);
        assert_eq!(text(&[0xE9]), "é");
        assert_eq!(latin1_bytes("✓"), b"?");
    }

    #[test]
    fn gshort_and_gint3_round_trip() {
        use seihai_core::protocol::{read_gint3, read_gshort};

        let mut v = Vec::new();
        push_gshort(&mut v, 130);
        push_gint3(&mut v, 0x1F_FFFF);
        assert_eq!(read_gshort(&v).unwrap(), 130);
        assert_eq!(read_gint3(&v[2..]).unwrap(), 0x1F_FFFF);
    }
}
