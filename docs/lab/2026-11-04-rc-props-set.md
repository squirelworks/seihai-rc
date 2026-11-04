# Lab note: props by account and props set, 2026-11-04

Two runs on the same loopback setup: the `seihai-gserver-v2` fork
(`seihai` at `1d7843a`) and `seihai-listserver` (`main`), in a network
namespace whose only interface is loopback. The staff account was
`labrc`. The target was `test1`, a plain lab account. Only lab passwords
were used. The capture (pcap) is private and kept out of this repo.

## 1. A real RC (capture 2)

Client: `RemoteControl.exe` from the Graal Reborn 2.22.0 pack (login
token `GSERV024`), under Wine. From the accounts window it listed the
accounts, opened Edit Attributes for `test1` while `test1` was offline,
set gralats from 0 to 3, added the NPC script flag `labflag=1`, applied,
and opened Edit Attributes again. It then opened Edit Account, set an
e-mail address and applied.

### Props by account

| Dir | Id | Payload | Seen |
|---|---|---|---|
| RC to server | 74 | GSTRING account, then one byte 0x20 (an empty GSTRING) | `%test1 ` |
| server to RC | 72 | as for 73, with player id 0 | `  %test1$main` and 143 bytes of props, flag count 0, chest count 0, 3 weapons |

### Props set

The 76 was GSTRING `test1`, GSTRING `main`, a 125-byte props block, then:

| Bytes | Meaning |
|---|---|
| `20 21` | GSHORT 1: one flag |
| `29` `labflag=1` | GSTRING, the flag |
| `20 20` | GSHORT 0: no opened chests |
| `23` and three GSTRINGs | 3 weapons: `bomb`, `bow`, `-gr_movement` |
| `20` | GCHAR 0 |

The props were the same list as in the first capture
(`docs/lab/2026-11-04-rc-player-admin.md`): 1 to 11, 13, 15, 16, 18, 20
to 23, 26 to 29, 32 and 35. Prop 7 went as 1, 21 to 23 went empty and 29
(online seconds) went as 0. The fork saved gralats 3 and the flag, and
kept online seconds at 30. The 72 that came back had a flag count of 0,
and the RC showed no flags.

### Accounts (stage 3)

| Dir | Id | Payload | Seen |
|---|---|---|---|
| RC to server | 72 | GSTRING name pattern, GSTRING conditions | `%`, empty |
| server to RC | 70 | a GSTRING per account | the six lab account files |
| RC to server | 77 | the account as the rest of the line | `test1` |
| server to RC | 73 | GSTRING account, GSTRING password (empty), GSTRING e-mail, three GCHARs, GSTRING admin worlds, two more bytes | e-mail empty, then the one set; all three GCHARs 0; worlds `main` |
| RC to server | 78 | GSTRING account, GSTRING password (empty), GSTRING e-mail, three GCHARs, GSTRING admin worlds, one more byte | e-mail set, rest as read |

The dialog has banned, load only, admin level, ban time and ban reason.
Which byte is which needs a run that changes one at a time.

### List RCs

The RC sent no 65 from the player list, the accounts window or their
menus. On login, the fork sent the RC chat line `Currently online: labrc`
by itself.

## 2. seihai-rc

`seihai-rc` from this branch, with `test1` offline, then online through
a scratch 2.22 player client:

| Command | Sent | Result |
|---|---|---|
| `players props test1` (offline) | 74 | `account test1 world main (not online)`, 25 props, flags 0, 3 weapons |
| `players props test1 --set gralats=5,bombs=7,hearts=2.5` (offline) | 74, 76, 74 | read back 5, 7 and 5 half hearts; the account file had `GRALATS 5`, `BOMBS 7`, `HP 2.5` and `ONSECS 30` |
| `players props test1 --set gralats=9,arrows=12` (online, player id 4) | 73, 76, 73 | read back 9 and 12 |
| `players props test1 --set nick=x` | nothing | refused before login |

Before the offline set the account file had `FLAG labflag=1` (put back by
hand for this check). The
fork's 72 did not carry it, so the 76 carried no flags, and after the set
the file had no `FLAG` line. A set through the real RC does the same.

Afterwards `test1` was set back to gralats 0, arrows 10, bombs 5 and 3
hearts.
