# Boundaries

Sturn owns **ephemeral coordination** — live claims, ops, topics,
asks over VCS state. Neighbors own theirs:

- **myco** — durable work memory (the `project/` ledger); sturn
  coordinates the moment, myco remembers the intent.
- **kuda** — code memory; `.kuda/` cards are that repo's concern.
- **tamias** — distribution; the `sturn` command is a tamias pack,
  releases run through `tarsi-serve`.
- **peregrin/hypha** — orchestration substrate; sturn is the
  coordination layer agents call directly.
