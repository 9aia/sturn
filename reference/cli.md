# CLI reference

Global flags: `--repo <dir>` (default cwd) · `--config <path>`.

| Command | What it does |
|---|---|
| `sturn init` | write `sturn.toml`; confirm ref namespaces |
| `sturn ws new <name> [--base]` | create workspace ref — no checkout |
| `sturn ws ls` | list workspaces |
| `sturn ws read <ws> <path>` | blob contents, straight from the ODB |
| `sturn ws hydrate <ws> <dir> [--path P] [--writeback]` | materialize view; --writeback diffs dir → ops |
| `sturn claim <glob> --ws <w> [--ttl N] [--force]` | advisory lock; exits 1 on live conflict |
| `sturn release <glob> --ws <w>` | drop a claim |
| `sturn claims` | live claims across every workspace |
| `sturn write <path> --ws <w> [--file F] [--intent T]` | append a write op (stdin → blob) |
| `sturn intent <text> --ws <w>` | record provenance |
| `sturn log [--ws <w>]` | replay the WAL as JSONL |
| `sturn snap <ws…> -m <msg>` | fold ops → commit; multi-ws merges into one |
| `sturn review <event-id> <approve\|reject> --ws <w>` | continuous review |
| `sturn push [--remote R]` | push heads + `refs/sturn/*` |
| `sturn daemon` | WAL → topic fan-out loop + `bus.sock` push |
| `sturn sub <topic>` | stream ops\|claims\|review\|presence as JSONL |
| `sturn mcp [--ws W]` | MCP stdio server; --ws emits joined/left presence |
| `sturn exec --ws <w> [--keep][--writeback] -- cmd…` | hydrate → run → clean; write-back as ops |
| `sturn join\|leave <ws>` | presence; leave sweeps actor claims |
| `sturn rename\|delete … --ws` | tree ops that keep intent |
| `sturn verify [--ws W]` | verify event sigs vs allowed_signers |
| `sturn compact [--ws W]` | drop pre-snapshot WAL history (archived) |
| `sturn bridge --to <url>` | snapshot ops → HMAC webhook (bupha CI) |
| `sturn mount <ws> <dir> [--rw]` | FUSE mount; --rw edits flow back as ops |
| `sturn deploy service\|env` | systemd unit / secrets skeleton |
| `sturn service enable\|disable\|status` | toggle the daemon unit (kept) — stop frees resources until re-enabled |
