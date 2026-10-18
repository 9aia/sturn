---
url: /sturn/reference/config.md
---
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
```

See `examples/sturn.toml.example` in the repo.
