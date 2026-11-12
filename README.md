# sturn

*Ephemeral coordination engine — claims/ops/topics over VCS state.*

Evented coordination over the git object store. The WAL is the
state, the working tree is a projection, the bus is a cache —
agents murmurate, not merge.

```
agent ──► claim(src/**) ──► WAL ──► topic ──► other agents see it NOW
        ──► write ops ────┘  (git refs)        not at merge, in ms
        ──► snap ──► real commit ──► push ──► GitHub (free, signed)
```

Git's object store was never the problem — *checkout
materialization*, *discover-at-merge conflicts*, and *the PR cliff*
were. Sturn keeps the ODB and the remotes, and replaces everything
in between with an op log and a coordination bus.

## Why

- **No clones, no worktrees.** A workspace is a pointer at a tree;
  a thousand agent workspaces cost nothing.
- **Conflicts prevented, not found.** `claim` publishes an advisory
  lock *before* the work happens; contenders learn in milliseconds.
- **Pub/sub is a cache, not authority.** Ops live in
  `refs/sturn/wal/*` — they replicate on ordinary `git push`.
  Daemon dies? `git fetch` rebuilds everything.
- **Continuous review.** Ops stream to reviewers as they happen;
  `snap` is the only thing that ever becomes a commit. PRs become
  a packaging format, not a gate.
- **Provenance is data.** Every op is attributed (`actor`) and
  carries `intent` — who changed it, what changed, why.

## Install

```sh
cargo install --git https://github.com/9aia/sturn sturn   # today
cargo install sturn                                      # post-0.1.0
```

Platform: **Linux** for the daemon (systemd); the CLI works
anywhere git does.

## Quickstart

```sh
cd your-repo
sturn init                          # refs namespace + sturn.toml

sturn ws new agent-7                # virtual workspace — no checkout
sturn claim "src/**" --ws agent-7   # advisory lock, conflicts NOW
echo "…" | sturn write src/x.rs --ws agent-7 --intent "fix precedence"
sturn snap agent-7 -m "fix parser precedence"   # → real git commit
sturn push                          # heads + refs/sturn/* ride along
```

The daemon fans WAL appends onto topics other actors subscribe to:

```sh
sturn daemon            # WAL → repo.<id>.{ops,claims,review,presence}
sturn deploy service    # systemd user unit
```

## Commands

`init` · `ws new/ls/read/paths/grep/hydrate` · `claim/release/claims` ·
`write`/`rename`/`delete`/`intent` · `log`/`compact` ·
`snap` (multi-ws merge) · `review`/`verify` ·
`ask`/`asks`/`answer`/`cancel`/`wait` · `push` ·
`daemon`/`sub`/`bridge` · `mount` · `exec` · `join`/`leave` ·
`deploy` — full table in [docs](https://9aia.com/sturn/reference/cli.html).
The agent surface is generated, not hand-rolled: `tamias-driver-sturn`
exposes every bounded verb as a `cli.<verb>` op (and MCP tool via the
pavo surface drivers) — `sturn mcp` is gone.

The `sturn` command also ships as a tamias pack (`libraries/sturn`) —
a generated `bins[]` shim → `tamias-driver-sturn cli` → the engine
lib's `run_from` with inherited stdio (`scripts/pack.sh` stages the
driver under `drivers/bin/<triple>/`). With `tamias` installed:

```sh
tamias install --allow-drivers ./libraries/sturn   # deploys `sturn`
```

## vs git / jj / AVCS

| | git | jj | sturn |
|---|---|---|---|
| State | working tree + index | op log (local) | op log (**shared**, in refs) |
| New workspace | worktree = full checkout | cheap | a pointer |
| Coordination | none | none | claims + topics |
| Merge | 3-way at merge time | same | same (ops are metadata) |
| Review | PR cliff | PR cliff | streamed verdicts |

jj's op log is per-clone; sturn's is replicated and published.
AVCS shares the *op-graph as truth* idea; sturn keeps merges on
git trees and adds the live coordination layer.

Works unchanged on **colocated jj** (`jj git init`) — sturn only
ever touches `.git`, and `refs/sturn/*` never maps to a bookmark.
Non-colocated repos hide the git dir under `.jj/` and aren't
supported.

## Status

Scaffold. Tasks, milestones, and decisions live in the `project/`
worktree — `myco status --prompt` for the board.

## License

MIT
