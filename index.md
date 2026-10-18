---
layout: home
hero:
  name: sturn
  text: agents murmurate, not merge
  tagline: Evented coordination over the git object store — the WAL is the state, the tree is a projection, the bus is a cache.
  actions:
    - theme: brand
      text: Get started
      link: /getting-started
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
