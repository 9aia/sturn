+++
id = "ADR-0002"
title = "ADR-0001 — evented coordination over the git object store"
status = "open"
priority = "medium"
+++

# ADR-0001 — evented coordination over the git object store

## Context

Git's object store is fine; the *workflow* around it costs in the
agent era: clone/worktree materialization per parallel task,
conflicts discovered at merge instead of prevented, PR review as a
batch cliff. Agents don't need POSIX checkouts — they need
read/write/glob/grep and a signal when a sibling touches their
files.

## Decision

- **The WAL is the state.** Ops (`write`/`rename`/`delete`/`claim`/
  `release`/`intent`/`snapshot`/`review`/`presence`) append to
  `refs/sturn/wal/<ws>` — each event a blob, each append a commit,
  CAS on the ref. Replicated by ordinary git push/fetch.
- **The bus is a cache.** Topics `repo.<id>.{ops,claims,review,
  presence}` accelerate delivery to live subscribers; catch-up
  replays the WAL. Broker death loses nothing.
- **Workspaces are pointers.** `refs/sturn/ws/<name>` → commit.
  No materialization; hydration is opt-in for POSIX-bound tools.
- **Merges stay git's.** Ops are metadata — claims prevent most
  collisions pre-work; `snap` squashes ops into ordinary commits.
- **Ports/impls** (bupha convention): `sturn-{wal,bus,ws,vcs,
  config,secret,service}` ports; `-git`, `-redb`, `-toml`,
  `-keyring`, `-dotenv`, `-systemd` impls. Single-file crates,
  dependency-light, sync, no async runtime.

## Consequences

- Free signed hosting (GitHub) and familiar history survive; the
  repo is legible to plain git.
- Trusted-swarm scope only: attribution is `actor`, auth is the
  git transport's. Signed ops and review gates are backlog.
- Op log growth needs compaction (post-M1).
- A daemon is new ops burden vs. dumb git — mitigated because the
  system degrades to git-sync-only without it.
