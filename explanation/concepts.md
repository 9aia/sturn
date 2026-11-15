# Concepts

The domain vocabulary, each term defined once. The op-level
contract lives in [Reference: ops](../reference/ops).

## WAL (write-ahead log)

`refs/sturn/wal/<ws>` — the append-only op log per workspace, stored
as ordinary git objects. **The WAL is the state**: anything
derivable from it (bus cursors, materialized views, live claims) is
a rebuildable cache.

## Op

The unit of coordination: `write`, `rename`, `delete`, `claim`,
`release`, `intent`, `review`, `ask`/`answer`, `presence`. Ops are
metadata about work — they never encode merge semantics. Content
payloads become ODB blobs; ops reference them.

## Workspace (`ws`)

A named pointer — `refs/sturn/ws/<name>` → a commit — plus that
workspace's WAL. Zero materialization: no clone, no worktree, no
checkout. `sturn ws new` costs a ref.

## Claim

An advisory lock on a resource URI (`fs:src/**`, `dev:display0`,
`sess:T-012`) published *before* edits, with a TTL. Conflicts are
learned in milliseconds on the claims topic — prevention, not
merge-time discovery. `sturn check` probes the conflict set;
`--fence` asks whether your claim was superseded.

## Topic / bus

`repo.<id>.{ops,claims,review,presence,asks}` — the pub/sub cache
the daemon fans WAL events onto. **A cache, never authority**: when
the daemon is down, ops still sync through git push/fetch and
subscribers replay the WAL.

## Snap

The bridge back to ordinary git: folds a workspace's ops into a
real commit on the branch. Multiple workspaces merge into ONE
commit — rename-aware, semantic conflicts warn and later ops win.

## Review / ask

`review <op-id> approve|reject` is itself an attributed op —
per-op approval that streams *before* commit time. `[review]
require = true` makes `snap` refuse unreviewed ops. `ask` opens a
pending-response question any actor may answer; `wait` blocks on
the verdict.

## Actor

Who you are on the ops you append — `STURN_ACTOR` env or config.
Every op is attributed and (optionally) signed; `sturn verify`
checks signatures against `allowed_signers`.

## Hydrate / mount / exec

The three ways to get real files: `ws hydrate`/`exec` materialize
the ws *view* (tip + uncommitted ops) to a dir/tmpfs;
`sturn mount --rw` exposes it as a FUSE filesystem where POSIX
writes become ops. See [model](model.md).
