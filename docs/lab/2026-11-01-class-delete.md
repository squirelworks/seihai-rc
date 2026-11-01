# Lab note: class delete, 2026-11-01

Client: `seihai-rc` on this branch (`nc class delete`). No original RC or
NPC-Control binary was run.

Server: a fake NPC-server in `tests/nc_commands.rs`, bound on the loopback
address at an ephemeral port. The fork was not asked. Class delete is not
in the MIT gserver.

What was done: `nc class delete Guard` sends id 119 and waits for id 188.
The name is a GSTRING, the same bytes class set writes in front of the
script. A reply whose payload is the bare name `Guard` is not accepted.

Before: `nc class delete` printed that the name still needed a lab check
and sent nothing. Id 188 was kept as raw bytes.

After: the command prints `deleted Guard` when id 188 is a GSTRING of that
name, and the stored class list drops it. A bare name is not sent and is
not read. A script that contains a line break is still not sent.

Logs: the test binary's stdout from `cargo test --test nc_commands
class_list_get_set_and_delete`. No pcap.
