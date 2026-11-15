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

## Boundaries

Sturn owns **ephemeral coordination** — live claims, ops, topics,
asks over VCS state. Neighbors own theirs:

- **myco** — durable work memory (the `project/` ledger); sturn
  coordinates the moment, myco remembers the intent.
- **kuda** — code memory; `.kuda/` cards are that repo's concern.
- **tamias** — distribution; the `sturn` command is a tamias pack,
  releases run through `tarsi-serve`.
- **peregrin/hypha** — orchestration substrate; sturn is the
  coordination layer agents call directly.

## Where it sits

In the org's layering — **tamias** (package & content manager) →
**engines** (myco records, kuda code maps, mimus proofs, sepia
sessions, hypha substrate, tarsi releases, cicada schedules, cypera
docs, scarab config) → **orchestration** (peregrin campaigns) →
**surfaces** (pavo views, MCP fronts, agent harnesses).

Sturn is an **engine** — the coordination substrate every agent
calls directly, below orchestration. It isn't orchestrated itself:
agents `claim`/`write`/`snap` against the WAL, the daemon only
fans events onto topics, and peregrin campaigns can consume sturn
ops as events without sturn knowing they exist.
