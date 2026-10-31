---
url: /sturn/how-to/daemon.md
---
# Run the daemon

`sturn daemon` polls WAL heads and fans new events onto topics
(`repo.<id>.ops|claims|review|presence`) in the embedded bus store.

```sh
sturn daemon                      # foreground
sturn deploy service              # systemd user unit, Restart=always
sturn service disable             # stop now + skip at login (unit kept)
sturn service enable              # back on, now and at login
sturn service status              # enabled=/active= probe
sturn service restart             # bounce in place
sturn service logs -f             # journal tail; -f follows
sturn destroy service             # disable + remove the unit file
```

## If the daemon is down

Nothing is lost. The bus is a cache — ops still live in
`refs/sturn/wal/*` and replicate via `git push`/`git fetch`.
Subscribers catch up by replaying the WAL. The daemon only removes
latency; it never owns truth.

## Consumers today

* `sturn claims` — fold all WALs into live advisory locks
* M2: SSE/unix-socket subscribers; M3: the MCP `subs` tool
