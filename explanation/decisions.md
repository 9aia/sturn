---
url: /sturn/explanation/decisions.md
---
# Decisions

ADR-lite: the load-bearing decisions, each with the alternative it
beat and the consequence it carries. Design history lives in the
`project/` myco ledger.

## The WAL is the state

**Decision:** ops live in `refs/sturn/wal/*` as ordinary git
objects; the bus, cursors, and materialized views are caches.

* *Alternative:* a dedicated database (or the bus) as authority.
* *Consequence:* replication is plain `git push`/`fetch` — signed
  by git's machinery, no new sync protocol. Cost: everything
  reconstructible must *stay* reconstructible; the rule is
  enforced by review.

## Ops are metadata; merges stay git's

**Decision:** coordination ops never encode merge semantics —
claims prevent collisions, `snap` produces ordinary commits.

* *Alternative:* CRDT merge for the tree, or a custom merge driver.
* *Consequence:* see [Why no CRDT for code](no-crdt.md) — the log
  converges deterministically, the tree needs decisions, and
  claims make most collisions never happen.

## Workspaces are pointers

**Decision:** `refs/sturn/ws/<name>` is a ref, not a checkout — a
thousand agent workspaces cost nothing.

* *Alternative:* git worktrees or clones per agent.
* *Consequence:* parallelism scales to the ref namespace; real
  files materialize only on demand (hydrate/exec/mount) for the
  tools that genuinely need POSIX.

## Claims before edits

**Decision:** advisory locks publish *before* work starts —
coordination precedes the edit, not the merge.

* *Alternative:* detect conflicts at merge/snap time.
* *Consequence:* contenders learn in milliseconds and pick
  different resources; the failure mode git taught us (discovering
  the collision at a three-way merge) is designed out.

## Continuous review over the PR gate

**Decision:** every op streams through the WAL and can carry a
`review` verdict; `[review] require` gates `snap` per-op.

* *Alternative:* one big diff reviewed at PR time.
* *Consequence:* verdicts land *before* commit time, per op, from
  any actor (agents review agents); PRs become packaging, not the
  gate.
