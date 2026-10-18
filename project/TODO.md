# TODO — milestone-gated

## M1 — WAL round-trip (the core proof)
- [x] `sturn init / ws new / write / log / snap` — ops in git refs,
      snap produces a real commit
- [ ] e2e test: init → write → snap → clone → WAL intact
- [ ] `snap` only applies ops since last `Snapshot` marker
- [ ] WAL compaction: squash pre-snapshot events into a checkpoint

## M2 — bus + claims (the delta)
- [ ] daemon: publish only WAL delta (currently republishes tail)
- [ ] subscription: unix socket / SSE for push delivery
- [ ] claim TTL sweep + `release` on actor death (presence)
- [ ] bus: redb key prefix index (stop full-table scans)

## M3 — agent surface
- [ ] sturn-mcp: real MCP server over the 9 tools
- [ ] `ws tip` resolves WAL overlay (uncommitted ops visible in read)
- [ ] agent e2e: two writers, one claim collision, zero merges

## M4 — hydration
- [ ] tmpfs overlay materializer (only touched paths)
- [ ] `cargo check` inside a virtual ws
- [ ] FUSE impl for humans/GUIs (last — git works meanwhile)

## M5 — dogfood
- [ ] sturn coordinates agents editing sturn
- [ ] bupha CI consumes sturn topics instead of webhooks
