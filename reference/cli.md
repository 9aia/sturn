# CLI reference

Global flags: `--repo <dir>` (default cwd) · `--config <path>`.

| Command | What it does |
|---|---|
| `sturn init` | write `sturn.toml`; confirm ref namespaces; note colocated jj |
| `sturn ws new <name> [--base]` | create workspace ref — no checkout |
| `sturn ws ls` | list workspaces |
| `sturn ws read <ws> <path>` | blob contents at the live view — tip + uncommitted ops |
| `sturn ws paths <ws> [glob]` | effective path listing (tip + pending ops), glob-filtered |
| `sturn ws grep <ws> <substr> [--path glob]` | substring search → `path:line: text` |
| `sturn ws hydrate <ws> <dir> [--path P] [--writeback]` | materialize view; --writeback diffs dir → ops |
| `sturn claim <uri> --ws <w> [--ttl N] [--force]` | advisory lock on a resource URI — `fs:` glob (`src/**`; bare glob means `fs:`) or opaque (`dev:display0`, `net:13000` or range `net:13000-13100`, `sess:T-012`, `cap:role`); prints the fencing token; exits 1 on live conflict |
| `sturn release <uri> --ws <w>` | drop a claim |
| `sturn claims` | live claims across every workspace — uri, holder, expiry, fence |
| `sturn check <uri> [--fence N] [--watch]` | list live claims conflicting with a URI; exits 1 when contested. `--fence N` reports only claims outranking N — the held-claim staleness probe. `--watch` streams conflict-set changes via WAL poll |
| `sturn write <path> --ws <w> [--content S] [--file F] [--intent T]` | append a write op (--content / --file / stdin → blob) |
| `sturn intent <text> --ws <w>` | record provenance |
| `sturn log [--ws <w>] [--topic T] [--limit N] [--wait-ms MS]` | replay the WAL as JSONL; topic/limit filter, wait-ms long-polls |
| `sturn snap <ws…> -m <msg>` | fold ops → commit; multi-ws merges into one — rename-aware; conflicts → stderr |
| `sturn review <event-id> <approve\|reject> --ws <w>` | continuous review |
| `sturn ask <subject> --ws <w> [--target T] [--option O…] [--ttl N]` | open a pending ask — event id is the handle |
| `sturn asks [--all] [--json]` | pending asks; --all adds resolved |
| `sturn answer <ask-id> <verdict> --ws <w>` | close an ask with a verdict |
| `sturn cancel <ask-id> --ws <w>` | withdraw an open ask |
| `sturn wait <ask-id> [--timeout N] [--poll MS]` | block until resolved; exit 0 answered · 2 cancelled · 3 expired · 4 timeout |
| `sturn push [--remote R]` | push heads + `refs/sturn/*` |
| `sturn daemon` | WAL → topic fan-out loop + `bus.sock` push |
| `sturn sub <topic>` | stream ops\|claims\|review\|presence\|asks as JSONL |
| — | the agent surface is generated: `tamias-driver-sturn` exposes every bounded verb as a `cli.<verb>` op / MCP tool (`tamias-driver-mcp sturn` or the driver's `mcp` feature) |
| `sturn exec --ws <w> [--keep][--writeback] -- cmd…` | hydrate → run → clean; write-back as ops |
| `sturn join\|leave <ws>` | presence; leave sweeps actor claims |
| `sturn rename\|delete … --ws` | tree ops that keep intent |
| `sturn verify [--ws W]` | verify event sigs vs allowed_signers |
| `sturn compact [--ws W]` | drop pre-snapshot WAL history (archived) |
| `sturn bridge --to <url>` | snapshot ops → HMAC webhook (CI ingest — bupha is the CI repo) |
| `sturn mount <ws> <dir> [--rw]` | FUSE mount; --rw edits flow back as ops |
| `sturn deploy service\|env` | systemd unit / secrets skeleton |
| `sturn service enable\|disable\|status\|restart\|logs [-f]` | toggle/bounce the daemon unit (kept); logs tails the journal |
| `sturn destroy service` | disable + remove the systemd user unit |
