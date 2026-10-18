# Your first swarm

Goal: two agents editing one repo, zero checkouts, one prevented
conflict.

## Setup

```sh
git init demo && cd demo
git commit -qm init --allow-empty
sturn init
sturn daemon &            # or: sturn deploy service
```

## Two workspaces

```sh
sturn ws new agent-a
sturn ws new agent-b
```

Both point at `HEAD`. Nothing was written to disk — no worktree,
no index, no checkout.

## Claim before you edit

```sh
STURN_ACTOR=agent-a sturn claim "src/**" --ws agent-a
# claimed src/** (ws agent-a, ttl 3600s)

STURN_ACTOR=agent-b sturn claim "src/lib.rs" --ws agent-b
# conflict: agent-a claimed src/** (ws agent-a, expires …)
# sturn: claim denied — --force to override
```

Agent-b learns in milliseconds that `src/` is taken and picks
`tests/**` instead. In git, both would have edited the file and
discovered it at merge.

## Write ops, snap, repeat

```sh
echo '…' | sturn write src/lib.rs --ws agent-a
echo '…' | sturn write tests/lib.rs --ws agent-b
sturn snap agent-a -m "a: lib"   # → real commit on main
sturn snap agent-b -m "b: test"  # → real commit on main
```

`git log` shows two ordinary signed-able commits. `git for-each-ref
refs/sturn/*` shows the coordination layer GitHub never needed to
understand.
