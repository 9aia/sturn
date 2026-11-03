# sturn — project index

Evented coordination over the git object store. One screen:

- **State**: append-only op log in `refs/sturn/wal/<ws>` — git
  objects, replicated by ordinary push/fetch.
- **Coordination**: claims (advisory locks), ops, review verdicts,
  presence — published to topics by the daemon.
- **Truth ordering**: WAL > bus > projections. The bus is a cache;
  git refs are authority.
- **Boundary**: merges stay on git trees. Ops never introduce new
  merge semantics — they carry intent and coordination.

- `TODO.md` — milestone-gated work (M1…M5)
- `BACKLOG.md` — unscheduled ideas
- `adrs/` — why it's shaped this way
