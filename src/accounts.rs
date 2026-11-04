//! Stage 3: accounts. List (72 → 70), get (77 → 73), set (78), add (70)
//! and delete (71).
//!
//! Every layout follows the 2026-11-04 loopback captures of a real RC
//! (`GSERV024`) against the gserver-v2 fork, in which one account field was
//! changed per apply: `docs/lab/2026-11-04-rc-accounts.md`. Each builder
//! has a byte-exact unit test and a case in `tests/fake_servers.rs`.

use crate::wire::{
    gchar, gdetokenize, gtokenize, latin1_bytes, message, push_gstring, read_gstring, text,
};

/// Client to server ids for stage 3 (ids are wire facts).
pub mod pli {
    pub const RC_ACCOUNTADD: u8 = 70;
    pub const RC_ACCOUNTDEL: u8 = 71;
    pub const RC_ACCOUNTLISTGET: u8 = 72;
    pub const RC_ACCOUNTGET: u8 = 77;
    pub const RC_ACCOUNTSET: u8 = 78;
}

/// Server to client ids for stage 3.
pub mod plo {
    pub const RC_ACCOUNTLISTGET: u8 = 70;
    pub const RC_ACCOUNTGET: u8 = 73;
}

/// The longest text one GSTRING holds.
const GSTRING_MAX: usize = 223;

/// The largest value a GCHAR holds.
const GCHAR_MAX: u8 = 0xDF;

/// Why a builder refused to produce bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountError {
    /// A line feed would end the message (the field is named).
    LineBreak(&'static str),
    /// An empty account name.
    NoAccount,
    /// A text longer than one GSTRING holds (the field is named).
    TooLong(&'static str),
    /// An admin level over what one GCHAR holds.
    BadLevel(u8),
}

impl std::fmt::Display for AccountError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AccountError::LineBreak(what) => write!(f, "{what} contains a line break"),
            AccountError::NoAccount => write!(f, "the account name is empty"),
            AccountError::TooLong(what) => {
                write!(f, "{what} is longer than {GSTRING_MAX} bytes")
            }
            AccountError::BadLevel(n) => {
                write!(f, "admin level {n} is over {GCHAR_MAX}")
            }
        }
    }
}

impl std::error::Error for AccountError {}

fn field(what: &'static str, value: &str) -> Result<Vec<u8>, AccountError> {
    if value.contains(['\n', '\r']) {
        return Err(AccountError::LineBreak(what));
    }
    let b = latin1_bytes(value);
    if b.len() > GSTRING_MAX {
        return Err(AccountError::TooLong(what));
    }
    Ok(b)
}

fn account_bytes(account: &str) -> Result<Vec<u8>, AccountError> {
    if account.is_empty() {
        return Err(AccountError::NoAccount);
    }
    field("the account name", account)
}

/// Id 72, `PLI_RC_ACCOUNTLISTGET`: GSTRING name pattern (`%` matches
/// anything), GSTRING conditions (capture: `%` and an empty one).
pub fn account_list(pattern: &str, conditions: &str) -> Result<Vec<u8>, AccountError> {
    let mut payload = Vec::new();
    push_gstring(&mut payload, &field("the pattern", pattern)?);
    push_gstring(&mut payload, &field("the conditions", conditions)?);
    Ok(message(pli::RC_ACCOUNTLISTGET, &payload))
}

/// Read id 70: one GSTRING per account, to the end of the message. No
/// match is an empty message.
pub fn parse_account_list(payload: &[u8]) -> Option<Vec<String>> {
    let mut i = 0;
    let mut names = Vec::new();
    while i < payload.len() {
        names.push(text(&read_gstring(payload, &mut i)?));
    }
    Some(names)
}

/// Id 77, `PLI_RC_ACCOUNTGET`: the account as the rest of the line.
pub fn account_get(account: &str) -> Result<Vec<u8>, AccountError> {
    Ok(message(pli::RC_ACCOUNTGET, &account_bytes(account)?))
}

/// Id 71, `PLI_RC_ACCOUNTDEL`: the account as the rest of the line.
pub fn account_delete(account: &str) -> Result<Vec<u8>, AccountError> {
    Ok(message(pli::RC_ACCOUNTDEL, &account_bytes(account)?))
}

/// The fields of the RC's account dialog that go on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Account {
    pub account: String,
    pub email: String,
    pub banned: bool,
    pub load_only: bool,
    pub admin_level: u8,
    pub admin_worlds: String,
    /// The ban reason, one item per line.
    pub ban_reason: Vec<String>,
}

/// Read id 73: GSTRING account, GSTRING password (always empty from the
/// fork), GSTRING e-mail, GCHAR banned, GCHAR load only, GCHAR admin level,
/// GSTRING admin worlds, one byte, then the ban reason tokenized as a
/// GSTRING. The byte before the reason was 0x20 in every capture and the
/// RC never sends it back; it is most likely the ban time. A reply where it
/// is not 0x20 is not read, as its width is not known.
pub fn parse_account(payload: &[u8]) -> Option<Account> {
    let mut i = 0;
    let account = text(&read_gstring(payload, &mut i)?);
    read_gstring(payload, &mut i)?;
    let email = text(&read_gstring(payload, &mut i)?);
    let flags = payload.get(i..i + 3)?;
    i += 3;
    let flag = |b: u8| b.wrapping_sub(0x20);
    let admin_worlds = text(&read_gstring(payload, &mut i)?);
    if *payload.get(i)? != 0x20 {
        return None;
    }
    i += 1;
    let reason = text(&read_gstring(payload, &mut i)?);
    Some(Account {
        account,
        email,
        banned: flag(flags[0]) != 0,
        load_only: flag(flags[1]) != 0,
        admin_level: flag(flags[2]),
        admin_worlds,
        ban_reason: gdetokenize(&reason),
    })
}

/// The 78 / 70 payload: GSTRING account, GSTRING password (empty leaves it
/// as it is), GSTRING e-mail, GCHAR banned, GCHAR load only, GCHAR admin
/// level, GSTRING admin worlds, then the ban reason tokenized as a GSTRING.
/// No ban time: the RC sent none even with one typed in.
fn account_payload(a: &Account, password: &str) -> Result<Vec<u8>, AccountError> {
    if a.admin_level > GCHAR_MAX {
        return Err(AccountError::BadLevel(a.admin_level));
    }
    let lines: Vec<String> = a.ban_reason.clone();
    for l in &lines {
        field("the ban reason", l)?;
    }
    let reason = if lines.is_empty() {
        Vec::new()
    } else {
        latin1_bytes(&gtokenize(&lines.join("\n")))
    };
    if reason.len() > GSTRING_MAX {
        return Err(AccountError::TooLong("the ban reason"));
    }
    let mut payload = Vec::new();
    push_gstring(&mut payload, &account_bytes(&a.account)?);
    push_gstring(&mut payload, &field("the password", password)?);
    push_gstring(&mut payload, &field("the e-mail address", &a.email)?);
    payload.push(gchar(u8::from(a.banned)));
    payload.push(gchar(u8::from(a.load_only)));
    payload.push(gchar(a.admin_level));
    push_gstring(&mut payload, &field("the admin worlds", &a.admin_worlds)?);
    push_gstring(&mut payload, &reason);
    Ok(payload)
}

/// Id 78, `PLI_RC_ACCOUNTSET`. An empty `password` leaves it as it is.
pub fn account_set(a: &Account, password: &str) -> Result<Vec<u8>, AccountError> {
    Ok(message(pli::RC_ACCOUNTSET, &account_payload(a, password)?))
}

/// Id 70, `PLI_RC_ACCOUNTADD`: the same layout as 78.
pub fn account_add(a: &Account, password: &str) -> Result<Vec<u8>, AccountError> {
    Ok(message(pli::RC_ACCOUNTADD, &account_payload(a, password)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test1() -> Account {
        Account {
            account: "test1".into(),
            email: "lab@example.invalid".into(),
            admin_worlds: "main".into(),
            ..Account::default()
        }
    }

    #[test]
    fn list_matches_the_capture() {
        // 72 + 0x20 = 'h'.
        assert_eq!(account_list("%", "").unwrap(), b"h!% ");
        assert_eq!(account_list("lab%", "").unwrap(), b"h$lab% ");
        assert_eq!(
            parse_account_list(b"&labadd%labrc").unwrap(),
            ["labadd", "labrc"]
        );
        assert_eq!(parse_account_list(b"").unwrap(), Vec::<String>::new());
        assert!(parse_account_list(b"&lab").is_none());
    }

    #[test]
    fn get_and_delete_send_the_bare_account() {
        // 77 + 0x20 = 'm', 71 + 0x20 = 'g'.
        assert_eq!(account_get("test1").unwrap(), b"mtest1");
        assert_eq!(account_delete("labadd").unwrap(), b"glabadd");
        assert_eq!(account_get(""), Err(AccountError::NoAccount));
        assert!(account_get("a\nb").is_err());
    }

    #[test]
    fn reply_73_reads_each_field() {
        let a = parse_account(b"%test1 3lab@example.invalid   $main  ").unwrap();
        assert_eq!(a, test1());
        let a = parse_account(b"%test1 3lab@example.invalid !\"$main ,\"lab reason\"").unwrap();
        assert!(!a.banned);
        assert!(a.load_only);
        assert_eq!(a.admin_level, 2);
        assert_eq!(a.ban_reason, ["lab reason"]);
        // A byte before the reason that is not 0x20: not read.
        assert!(parse_account(b"%test1 3lab@example.invalid   $main!  ").is_none());
        assert!(parse_account(b"%test1 3lab@ex").is_none());
    }

    #[test]
    fn set_matches_the_capture_one_field_at_a_time() {
        // 78 + 0x20 = 'n'. Each line is one apply in the capture.
        let mut a = test1();
        assert_eq!(
            account_set(&a, "").unwrap(),
            b"n%test1 3lab@example.invalid   $main "
        );
        a.admin_level = 2;
        assert_eq!(
            account_set(&a, "").unwrap(),
            b"n%test1 3lab@example.invalid  \"$main "
        );
        a.admin_level = 0;
        a.load_only = true;
        assert_eq!(
            account_set(&a, "").unwrap(),
            b"n%test1 3lab@example.invalid ! $main "
        );
        a.banned = true;
        assert_eq!(
            account_set(&a, "").unwrap(),
            b"n%test1 3lab@example.invalid!! $main "
        );
        let mut a = test1();
        a.ban_reason = vec!["lab reason".into()];
        assert_eq!(
            account_set(&a, "").unwrap(),
            b"n%test1 3lab@example.invalid   $main,\"lab reason\""
        );
        a.admin_worlds = "main,lab".into();
        assert_eq!(
            account_set(&a, "").unwrap(),
            b"n%test1 3lab@example.invalid   (main,lab,\"lab reason\""
        );
    }

    #[test]
    fn add_matches_the_capture() {
        // 70 + 0x20 = 'f'. The password here is a lab placeholder.
        let a = Account {
            account: "labadd".into(),
            email: "add@example.invalid".into(),
            admin_level: 1,
            admin_worlds: "all".into(),
            ..Account::default()
        };
        assert_eq!(
            account_add(&a, "pw").unwrap(),
            b"f&labadd\"pw3add@example.invalid  !#all "
        );
    }

    #[test]
    fn builders_refuse_what_does_not_fit() {
        let mut a = test1();
        a.admin_level = 0xE0;
        assert_eq!(account_set(&a, ""), Err(AccountError::BadLevel(0xE0)));
        let mut a = test1();
        a.email = "x".repeat(224);
        assert!(account_set(&a, "").is_err());
        let mut a = test1();
        a.ban_reason = vec!["a\rb".into()];
        assert!(account_set(&a, "").is_err());
        assert!(account_set(&test1(), "pw\n").is_err());
    }
}
