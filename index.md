---
layout: home
hero:
  name: sturn
  text: agents murmurate, not merge
  tagline: Ephemeral coordination engine — claims/ops/topics over VCS state
  actions:
    - theme: brand
      text: Get started
      link: /tutorials/getting-started
    - theme: alt
      text: Why sturn?
      link: /explanation/architecture
features:
  - title: Workspaces are pointers
    details: No clones, no worktrees, no checkout materialization. refs/sturn/ws/<name> is a ref to a commit — a thousand agent workspaces cost nothing.
  - title: Conflicts prevented, not found
    details: Claims publish advisory locks before edits happen. Contenders learn in milliseconds on the claims topic, not at a three-way merge twenty minutes later.
  - title: The bus is a cache
    details: Ops live in refs/sturn/wal/* as ordinary git objects — replicated by plain push/fetch, signed by git's own machinery. The pub/sub layer accelerates; it never owns truth.
  - title: Continuous review
    details: Ops stream to reviewers as they land. snap is the only thing that becomes a commit — the PR stops being a gate and becomes a packaging format.
---

## What it is

Sturn is ephemeral coordination for agents working one repo: claims,
ops, topics, and asks appended to a git-native WAL
(`refs/sturn/wal/*`), with workspaces as zero-cost pointers
(`refs/sturn/ws/*`) and a rebuildable pub/sub bus for live stream.

## Why

Parallel agents collide at merge time — twenty minutes into a
three-way merge you learn two of them edited the same file. Sturn
moves coordination *before* the edit: claims publish advisory
locks in milliseconds, ops stream to reviewers continuously, and
`snap` is the only thing that becomes a commit. The WAL rides
ordinary git push/fetch, so coordination replicates with zero new
infrastructure.

## Boundary

- **Owns:** live claims, ops, topics, asks — the moment-of-work
  coordination layer.
- **Doesn't own:** durable work memory ([myco](https://9aia.com/myco/)),
  code memory ([kuda](https://9aia.com/kuda/)), distribution
  ([tamias](https://9aia.com/tamias/)), orchestration
  ([peregrin](https://9aia.com/peregrin/)).

## Where to go next

| You want to… | Go to |
| --- | --- |
| Run your first swarm — two agents, one repo | [Tutorial](/tutorials/getting-started) |
| Do a task (hydrate, mount, daemon ops) | [How-to](/how-to/) |
| Understand the design (WAL, model, no-CRDT) | [Explanation](/explanation/) |
| Look up an op, verb, or config field | [Reference](/reference/) |
