# TODO — milestone-gated

## M1 — WAL round-trip (the core proof)
- [x] `sturn init / ws new / write / log / snap` — ops in git refs,
      snap produces a real commit
- [x] e2e test: init → write → snap → clone → WAL intact
- [x] `snap` only applies ops since last `Snapshot` marker
- [x] WAL compaction: `sturn compact` (pre-snapshot → `refs/sturn/archive/*`)

## M2 — bus + claims (the delta)
- [x] daemon: publish only WAL delta (`events_since` range walk)
- [x] subscription: unix socket `bus.sock`, `sturn sub <topic>` → JSONL
- [x] real glob matching (`*`, `**`) in claim conflicts
- [x] claim TTL + `join`/`leave` presence — leave sweeps actor claims
- [x] bus: seq-counter table — O(1) publish (poll still prefix-scans)

## M3 — agent surface
- [x] sturn-mcp: real MCP server over the tools (`sturn mcp`,
      newline JSON-RPC on stdio — initialize/tools/list/tools/call)
- [x] overlay reads: `ws_read`/`ws_glob`/`ws_grep` see uncommitted
      ops (tip + WAL since last Snapshot)
- [x] agent e2e: `mcp_agent_surface` — two ws, claim denial, snap

## M4 — hydration
- [x] overlay-aware hydrate: tip + uncommitted ops (`--path` filter)
- [x] `sturn exec --ws a -- cmd` — hydrate to tmpfs, run, clean up;
      e2e: cargo check fails on WAL-only code, passes on op fix
- [x] FUSE: `sturn mount` — read-only tree mount via fuser

## M4.5 — shipped with M4/M5
- [x] MCP `subs wait_ms` — long-poll live feed for agents
- [x] signed ops: `[signing] key` → ssh-keygen -Y sign, `sturn verify`
- [x] `sturn bridge` — snapshot ops → GitHub-shaped HMAC webhook to bupha
- [x] review gates: `[review] require = true` — snap refuses unreviewed tree ops
- [x] multi-repo daemon: `[bus] repos = [...]` fans out extra repos

## M5 — dogfood
- [x] sturn coordinates agents editing sturn — commit a2fb761
      authored entirely as ops, tested in the virtual ws, snapped;
      caught a real bug (rename resolving against base, not overlay)
- [ ] bupha CI consumes sturn topics instead of webhooks
