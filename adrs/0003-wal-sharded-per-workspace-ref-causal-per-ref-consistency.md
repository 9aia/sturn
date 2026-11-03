+++
id = "ADR-0003"
title = "WAL sharded per-workspace ref; causal (per-ref) consistency"
status = "done"
created = "2026-11-03"
priority = "medium"
+++

## Context

Agent-scale concurrency on a repo: many writers, frequent small ops,
coordination state that must replicate with the repository itself.
A repo-wide ordered log serializes unrelated writes (the shared-manifest
WAL anti-pattern); consensus across replicas adds quorum latency (3PC
anti-pattern).

## Decision

WAL ops are commits on per-workspace refs `refs/sturn/wal/<ws>`; appends
CAS that one ref only (retry on lost race). Per-ref serializability,
parallelism across refs — causal consistency. The bus is a cache; refs
are authority; `git push refs/sturn/*` replicates state for free.
No global point-in-time snapshot is provided or required.

## Consequences

- Unrelated workspaces never contend on a shared coordination point.
- Readers can observe ws refs at different times — acceptable; merges
  resolve at `snap`, ops are metadata.
- Fits any refs+objects host; scaled backends (e.g. OpenAI's Pando)
  absorb op-frequency push volume natively.

## Validation

Same tradeoff OpenAI's Pando landed on for the *serving* layer —
it explicitly rejected shared-manifest WAL and 3PC, and provides
causal consistency with per-ref serialization. sturn applies the
identical model one layer up, at coordination.
