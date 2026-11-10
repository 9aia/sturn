# Config reference

`sturn.toml` — searched at `$STURN_CONFIG`, `./sturn.toml`,
`~/.config/sturn/sturn.toml`. Unknown fields are rejected. Secrets
never live here.

```toml
[repo]
dir = "."              # required — the git repo being coordinated
id = "9aia/sturn"      # topic prefix; default: dir basename
branch = "main"        # snap target; default: main

[wal]
ref_prefix = "refs/sturn/wal"   # default

[bus]
store = "~/.local/state/sturn/bus.redb"   # default
poll_ms = 100                             # daemon fan-out interval
# repos = ["../other-repo"]             # extra repos to fan out

[review]
require = true         # snap refuses unreviewed tree ops

[signing]
key = "~/.ssh/id_ed25519"             # sign every event (ssh-keygen)
# allowed_signers = "..."

[actor]
name = "agent-7"       # default: $STURN_ACTOR → $USER → "anon"

[hooks]
# post_op = "kuda sync"  # or a list: ["kuda sync", "./scripts/notify"]
```

`[hooks] post_op` runs each command detached (`sh -c`, null
stdio, cwd = repo dir) after a successful verb that moved refs —
claim/release/join/leave, write/rename/delete/intent, asks,
review, compact, snap, push, `ws new`, `--writeback`/`--rw`
surfaces. Read verbs never fire it, a failed hook never fails
the op, and `$STURN_POST_OP` (one command) overrides the list.
Off by default — it's the event-driven way to re-index, e.g.
`post_op = "kuda sync"` replaces `kuda watch --poll-refs` when
sturn drives the writes.

See `examples/sturn.toml.example` in the repo.
