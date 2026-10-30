# sturn

*The murmuration layer for your repo.*

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

`init` · `ws new/ls/read/hydrate` · `claim/release/claims` ·
`write`/`rename`/`delete`/`intent` · `log`/`compact` ·
`snap` (multi-ws merge) · `review`/`verify` ·
`ask`/`asks`/`answer`/`cancel`/`wait` · `push` ·
`daemon`/`sub`/`bridge` · `mcp` · `mount` · `exec` · `join`/`leave` ·
`deploy` — full table in [docs](https://9aia.com/sturn/reference/cli.html).

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

## Status

Scaffold — M1 (WAL round-trip: `init`/`ws`/`write`/`snap`/`log`)
is the gate. See `project/TODO.md` for milestones and
`project/adrs/` for the shape.

## License

MIT
