# TODO — milestone-gated

## M1 — WAL round-trip (the core proof)
- [x] `sturn init / ws new / write / log / snap` — ops in git refs,
      snap produces a real commit
- [x] e2e test: init → write → snap → clone → WAL intact
- [x] `snap` only applies ops since last `Snapshot` marker
- [ ] WAL compaction: squash pre-snapshot events into a checkpoint

## M2 — bus + claims (the delta)
- [x] daemon: publish only WAL delta (`events_since` range walk)
- [x] subscription: unix socket `bus.sock`, `sturn sub <topic>` → JSONL
- [x] real glob matching (`*`, `**`) in claim conflicts
- [ ] claim TTL sweep + `release` on actor death (presence)
- [ ] bus: redb key prefix index (stop full-table scans)

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
- [ ] FUSE impl for humans/GUIs (last — git works meanwhile)

## M5 — dogfood
- [ ] sturn coordinates agents editing sturn
- [ ] bupha CI consumes sturn topics instead of webhooks
