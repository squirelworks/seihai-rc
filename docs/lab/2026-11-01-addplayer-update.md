# Lab note: ADDPLAYER repeat prints as `[update]`, 2026-11-01

Client: `seihai-rc` on this branch (`login`). No original RC binary was run.

Server: a fake gserver in `tests/fake_servers.rs` (`repeated_addplayer_is_an_update`),
bound on the loopback address at an ephemeral port. The gserver-v2 lab pack
is not in the agent environment, so this run does not speak to the fork.
The resend itself was already seen on the fork on 2026-10-31
(`docs/lab/2026-10-31-stage1.md`): a listed player's props change produced a
second `ADDPLAYER` for the same id, and `login` printed `[online]` both times.

What was done: after a type-6 login the fake sends `ADDPLAYER` for id 6,
the same id again with a new level and nickname, `DELPLAYER` 6, then
`ADDPLAYER` for id 6 once more.

Before: both `ADDPLAYER`s for an id already listed were `AddPlayer` events,
so `login` printed `[online]` again.

After: the first and the post-`DELPLAYER` message are `[online]`. The repeat
while the id is still listed is `[update]`, and the stored level and nickname
are the ones from that message. `cargo test` covers the list logic and the
fake session.

Logs: the test binary's stdout from `cargo test --test fake_servers
repeated_addplayer_is_an_update`. No pcap.
