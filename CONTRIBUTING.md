# Contributing

Issues and PRs are **disabled** — the project is agent-maintained.
Fork freely under MIT. (Yes, a VCS that coordinates without PRs,
maintained without PRs.)

## Layout

`crates/` — rust workspace. `sturn-core` is pure domain (op schema,
config). Ports are crates: `sturn-wal`, `sturn-bus`, `sturn-ws`,
`sturn-vcs`, `sturn-config`, `sturn-secret`, `sturn-service`. Impls:
`sturn-wal-git`/`sturn-git`/`sturn-ws-git` (git plumbing),
`sturn-bus-redb`, `sturn-config-toml`, `sturn-keyring`/`sturn-dotenv`,
`sturn-systemd`. `sturn-claim` is pure claim logic. `sturn-mcp` is
the agent tool contract. `sturn-cli` (bin `sturn`) is the
composition root.

## Rules of thumb

- New WAL/bus/ws backend = a new impl crate, not a core change.
- `cargo build --workspace` and `cargo test --workspace` must stay
  green.
- Keep the binary dependency-light and sync; no async runtime.
- Commits: conventional prefixes (`feat:`/`fix:`/`refactor:`/`docs:`/`chore:`).
- Never commit secrets — `sturn.toml` takes names, not values.

## Invariants

- **The WAL is the state.** Anything reconstructible from
  `refs/sturn/wal/*` must never be required elsewhere; the bus and
  workspaces are rebuildable caches.
- **Ops are metadata.** Merge semantics stay git's — no new merge
  theory in this repo.
- **Coordination before work.** Claims publish before edits, not
  after. Never add a mechanism that discovers conflicts at merge.

## Docs

Follow Diátaxis on the `docs` branch: `docs/tutorials/` (learn) ·
`docs/how-to/` (tasks) · `docs/reference/` (lookup) ·
`docs/explanation/` (why). The branch mounts as the `docs/` worktree.

## Releasing

Seed once with `sturn deploy release`… *(release loop pending —
see project/BACKLOG.md)*
