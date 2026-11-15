# Your first swarm

Two agents editing one repo, zero checkouts, one prevented conflict.
By the end you'll have claimed a resource before touching it,
landed ops as ordinary commits, and pushed the coordination layer
to the remote — the whole sturn loop.

## Install

The `sturn` command ships as a [tamias](https://9aia.com/tamias/)
pack — a generated shim over `tamias-driver-sturn`. With tamias:

```sh
tamias install 9aia:sturn@<version>          # org registry
# or from a checkout:
tamias install --allow-drivers ./libraries/sturn
```

A compat `sturn` bin also exists in `sturn-cli`, so a plain
`cargo install --git https://github.com/9aia/sturn sturn` works too.

## Set up a repo

```sh
git init demo && cd demo
git commit -qm init --allow-empty
sturn init            # writes sturn.toml, confirms ref namespaces
sturn daemon &        # WAL → repo.<id>.{ops,claims,…}; or:
sturn deploy service  # systemd user unit, Restart=always
```

`sturn init` also detects colocated jj repos — sturn works on them
unchanged. You should see `wrote sturn.toml` (or an unchanged
report) and the daemon logging topic fan-out.

## Two workspaces

Workspaces are pointers, not checkouts — `refs/sturn/ws/<name>` is
a ref to a commit:

```sh
sturn ws new agent-a
sturn ws new agent-b
sturn ws ls            # both point at HEAD; nothing hit disk
```

## Claim before you edit

Claims publish advisory locks *before* edits — that's the whole
point:

```sh
STURN_ACTOR=agent-a sturn claim "src/**" --ws agent-a
# claimed src/** (ws agent-a, ttl 3600s)

STURN_ACTOR=agent-b sturn claim "src/lib.rs" --ws agent-b
# conflict: agent-a claimed src/** (ws agent-a, expires …)
# sturn: claim denied — --force to override
```

Agent-b learns in milliseconds that `src/` is taken and picks
`tests/**` instead. In git, both would have edited the file and
discovered it at merge. `sturn check "src/lib.rs"` probes the same
conflict set without claiming; `--fence` asks "has my claim been
superseded?"

## Write ops, snap, push

Ops are the only write path — `write` content becomes an ODB blob:

```sh
echo 'fn helper() { 42 }' | STURN_ACTOR=agent-a sturn write src/lib.rs --ws agent-a --intent "add helper"
echo '…' | STURN_ACTOR=agent-b sturn write tests/lib.rs --ws agent-b
echo '…' | STURN_ACTOR=agent-b sturn rename tests/lib.rs tests/helper.rs --ws agent-b
sturn log --ws agent-a    # JSONL replay of the WAL
```

`snap` folds a workspace's ops into a real commit — several ws
merge into ONE commit, rename-aware:

```sh
sturn snap agent-a -m "a: lib"   # → ordinary commit on main
sturn snap agent-b -m "b: test" # → ordinary commit on main
git log --oneline                # two normal commits
```

Then publish heads *and* the coordination layer:

```sh
sturn push    # heads + refs/sturn/* → origin
```

## What you now know

- The WAL (`refs/sturn/wal/*`) is the state; the bus and workspaces
  are rebuildable caches — the daemon only removes latency.
- Claims prevent collisions; `snap` is the only thing that becomes
  a commit; merges stay git's.
- `STURN_ACTOR` names who you are on the ops you append.

## Go deeper

- [Hydrate a workspace](../how-to/hydrate.md) — materialize the ws
  view for compilers that need POSIX files.
- [Mount a workspace](../how-to/mount.md) — FUSE: tools write
  normally, sturn records ops.
- [Run the daemon](../how-to/daemon.md) — service lifecycle.
- [Architecture](../explanation/architecture.md) — the three-layer
  model and its invariants.
- [The model](../explanation/model.md) — virtual files, per-op
  approval, agents reviewing agents.
- [Reference: ops](../reference/ops.md) — the full op vocabulary.
