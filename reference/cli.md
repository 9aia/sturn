# CLI reference

Global flags: `--repo <dir>` (default cwd) · `--config <path>`.

| Command | What it does |
|---|---|
| `sturn init` | write `sturn.toml`; confirm ref namespaces |
| `sturn ws new <name> [--base]` | create workspace ref — no checkout |
| `sturn ws ls` | list workspaces |
| `sturn ws read <ws> <path>` | blob contents, straight from the ODB |
| `sturn ws hydrate <ws> <dir>` | materialize to a real dir |
| `sturn claim <glob> --ws <w> [--ttl N] [--force]` | advisory lock; exits 1 on live conflict |
| `sturn release <glob> --ws <w>` | drop a claim |
| `sturn claims` | live claims across every workspace |
| `sturn write <path> --ws <w> [--file F] [--intent T]` | append a write op (stdin → blob) |
| `sturn intent <text> --ws <w>` | record provenance |
| `sturn log [--ws <w>]` | replay the WAL as JSONL |
| `sturn snap <ws> -m <msg> [--branch B]` | fold ops into a real commit |
| `sturn review <event-id> <approve\|reject> --ws <w>` | continuous review |
| `sturn push [--remote R]` | push heads + `refs/sturn/*` |
| `sturn daemon` | WAL → topic fan-out loop + `bus.sock` push |
| `sturn sub <topic>` | stream ops\|claims\|review\|presence as JSONL |
| `sturn deploy service\|env` | systemd unit / secrets skeleton |
