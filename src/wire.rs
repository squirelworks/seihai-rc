//! Small encoders for the Graal wire types the RC sends.
//!
//! Every number on the wire is a "G" type: each byte carries 7 bits plus
//! `0x20`, so no byte is a line feed. A message is `GCHAR id` + payload +
//! `\n`. Decoding helpers come from `seihai-core`.

/// One GCHAR: `n + 0x20`.
pub fn gchar(n: u8) -> u8 {
    n.wrapping_add(0x20)
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
