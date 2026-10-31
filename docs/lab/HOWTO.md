# Lab setup for seihai-rc

Loopback only (`127.0.0.1`), in a VM, container or CI runner. Never point the
RC at a public server or use a real account.

1. Build `seihai-gserver-v2` (branch `seihai`) as its README says.
2. Copy its `bin/` folder somewhere to work in. In
   `servers/default/config/serveroptions.txt` set `serverip`, `serverinterface`,
   `localip` and `listip` to `127.0.0.1` and `upnp = false`.
3. Add an RC account: copy `servers/default/accounts/YOURACCOUNT.txt` to
   `labrc.txt`, replace the name inside, set `IPRANGE 127.0.0.1`, and add
   `labrc` to the `staff` option. Restart the server after editing `staff`.
4. Start the lab listserver (`listserver.py` from the handoff folder; it is in
   `seihai-listserver/docs/reference/` too), then `./gs2emu`. The fork checks RC
   logins with the listserver, so it must be connected.
5. Run:

   ```
   export SEIHAI_RC_PASSWORD=labpw   # the lab listserver accepts any password
   seihai-rc servers --listserver 127.0.0.1:14922 --account labrc
   seihai-rc players list --server 127.0.0.1:14802 --account labrc
   seihai-rc login --server 127.0.0.1:14802 --account labrc -v --for 10
   ```

6. For players in the list, log in Seihai clients (`seihai-mae --version 2.05`
   under Xvfb, or `seihai-ato --version 3.153 --no-window`), as in the
   handoff's lab-testing notes.

Watch out: if the fork refuses an RC login because the account is not staff,
it may rewrite that account file with defaults. Check `IPRANGE` again before
retrying.

Record each run in `docs/lab/YYYY-MM-DD-topic.md`: commits, clients, commands,
what happened before and after.
