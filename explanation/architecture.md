# Architecture

Three layers, strictly ordered by authority:

```
        ┌────────── evented layer ──────────┐
agents  │  ops: write/rename/claim/intent/… │
        │  virtual workspaces (ws = pointer) │
        │  bus: repo.<id>.{ops,claims,…}     │
        └──────┬─────────────────┬───────────┘
               │ append          │ append (offline)
        refs/sturn/wal/* ── git ODB ── refs/sturn/ws/*
               │
        normal git push/fetch → GitHub — free, signed, ordinary
```

## Invariants

1. **The WAL is the state.** Anything derivable from
   `refs/sturn/wal/*` is a cache (bus cursors, materialized views).
2. **Merges stay git's.** Ops never encode merge semantics — claims
   prevent collisions; `snap` squashes into ordinary commits.
3. **Coordination before work.** Claims publish *before* edits.
   Discovering a conflict at merge is the failure mode git taught us.

## What each piece owes

- `refs/sturn/wal/<ws>` — source of truth, replicates for free
- daemon + bus — latency reduction only; optional
- `refs/sturn/ws/<name>` — workspace namespaces; hydration opt-in
- `snap` — the bridge back to ordinary git history
