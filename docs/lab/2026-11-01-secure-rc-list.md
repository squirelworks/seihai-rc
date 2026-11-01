# Lab note: RC list over the secure handshake, 2026-11-01

Client: `seihai-rc` on this branch (`servers --secure-login --server-key`,
and `fetch_rc_list_secure`). No original RC binary was run.

Server: a fake listserver in `tests/fake_servers.rs`, bound on the loopback
address at an ephemeral port. It speaks `SEI00001` on id 5 and then the
same gen-5 list the classic RC list uses. seihai-listserver was not the
peer: it answers `SEI00001` on id 7, and this client does not send id 7.

What was done: C1 is id 5, the key byte, `SEI00001`, the RC token and the
client's X25519 key, with no list name. C2 seals the id 1 payload (account
and password GSTRINGs). The reply is one server, type tag `H`, plus the
status line. A different pinned key produces no C2. An id 4 instead of S1
is a refusal.

Before: the RC list sent the account in a gen-5 id 1 after a classic id 5
handshake. There was no secure form.

After: `--secure-login` prints the server from that list. The player-list
form (id 7 and a list name) is not sent. C1 is 62 bytes: id, key, token,
RC token, 43-byte key, line feed.

Logs: the test binary's stdout from `cargo test --test fake_servers
secure_rc_list`. No pcap.
