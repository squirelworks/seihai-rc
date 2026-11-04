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

/// Read tokenized text back into lines. A quoted item runs to its closing
/// quote (`""` inside is one `"`); an unquoted item runs to the next comma.
/// Spaces around an item are dropped. Empty text gives no lines.
pub fn gdetokenize(text: &str) -> Vec<String> {
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
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
