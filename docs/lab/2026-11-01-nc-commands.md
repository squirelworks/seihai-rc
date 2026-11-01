# Lab note: NPC-server discovery and NPC, class, weapon commands, 2026-11-01

Client: `seihai-rc` on this branch (`nc discover`, `nc npcs`, `nc npc`,
`nc classes`, `nc class`, `nc weapons`, `nc weapon`). No original RC or
NPC-Control binary was run.

Server: fake game servers and fake NPC-servers in `tests/nc_commands.rs`,
bound on the loopback address at ephemeral ports. The gserver-v2 lab pack
is not in the agent environment, so this run does not speak to the fork.

What was done: the client logs into the game server, sends id 94 for a
player whose nickname contains `(Server)`, reads id 79, and opens the
NPC-Control socket only when that text is `host,port` or `host port`. It
then lists NPC rows, reads and writes an NPC script, deletes an NPC, lists
and edits a class, and lists, reads, writes and deletes a weapon. Class
delete is refused before any socket is opened.

Before: NPC Control could log in when the address was already known. It
could not find that address, and it did not speak the list, get, set or
delete commands.

After: `cargo test` covers the comma and space address shapes, a colon
shape that does not open the NPC-Control socket, and each command against
the fake NPC-server. Payloads the notes still leave for a lab check (class
delete, a script that contains a line break, a weapon length that does not
fit the message) are not given invented bytes.

Logs: the test binary's stdout from `cargo test --test nc_commands`. No pcap.
