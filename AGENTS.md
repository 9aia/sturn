# Sturn

Ephemeral coordination engine — claims/ops/topics over VCS state.
The WAL is the state, the working tree is a projection, the bus is a
cache — agents murmurate, not merge. Rust workspace,
ports-and-adapters: `sturn-core` is pure domain; ports are traits,
impls are separate crates.

## Layout

| Crate | Role |
| --- | --- |
| `sturn-cli` | the `sturn` engine — lib + compat bin; composition root, every verb |
| `sturn-core` | pure domain — op schema, config, shared vocabulary; no I/O |
| `sturn-wal` / `sturn-wal-git` | op-log port + git impl (`refs/sturn/wal/*`) |
| `sturn-odb` | object-DB reads over git |
| `sturn-bus` / `sturn-bus-redb` | pub/sub cache port + redb impl (rebuildable, never authority) |
| `sturn-ws` / `sturn-ws-git` | workspace pointer port + git impl |
| `sturn-git` | snap bridge over the shared `lasius-vcs` port (no git2/gix) |
| `sturn-claim` / `sturn-ask` | pure fold logic over the WAL — advisory locks, asks |
| `sturn-config` | config via scarab-config-file |
| `sturn-secret` / `sturn-keyring` / `sturn-dotenv` | secrets port + impls |
| `sturn-service` / `sturn-systemd` | service-install port + systemd impl (Linux daemon) |
| `sturn-fuse` | FUSE mount surface |
| `sturn-driver` | `tamias-driver-sturn` — pavo view/ops driver; the ops manifest is the agent surface (generated MCP/CLI tools) |
| `libraries/sturn` | the tamias pack — `bins[]` shim → `tamias-driver-sturn cli`; `scripts/pack.sh` stages the driver payload |
| `project/` | the myco ledger worktree — ADRs, tasks, handoffs |

## Commands

```bash
cargo build --workspace
cargo test --workspace
scripts/coverage.sh    # cargo-llvm-cov HTML → target/llvm-cov/html;
                       # extra args pass through (--fail-under-lines 100)
scripts/pack.sh        # stage libraries/sturn driver payload
tarsi-serve release --dir . --yes
                       # operator-run: version bump + tag
                       # (dry-run without --yes)
```

The `sturn` command ships as the tamias pack in `libraries/sturn` —
install via `tamias install --allow-drivers ./libraries/sturn` or the
org registry ref (`9aia:sturn@<version>`).

## Boundary

Sturn owns **ephemeral coordination** — live claims, ops, topics over
VCS state. Neighbors own theirs:

- **myco** — durable work memory (the `project/` ledger); sturn
  coordinates the moment, myco remembers the intent.
- **kuda** — code memory; `.kuda/` cards are that repo's concern.
- **tamias** — distribution; the `sturn` command is a tamias pack,
  release/bump/publish run through `tarsi-serve`.
- **peregrin/hypha** — orchestration substrate; sturn is the
  coordination layer agents call directly.

## Rules

- **The WAL is the state.** Anything reconstructible from
  `refs/sturn/wal/*` must never be required elsewhere; the bus and
  workspaces are rebuildable caches.
- **Ops are metadata.** Merge semantics stay git's — no new merge
  theory in this repo.
- **Coordination before work.** Claims publish before edits, not
  after. Never add a mechanism that discovers conflicts at merge.
- New WAL/bus/ws backend = a new impl crate, not a core change.
- Keep the binary dependency-light and sync; no async runtime.
- Never commit secrets — `sturn.toml` takes names, not values.
- Commits: conventional prefixes (`feat:`/`fix:`/`refactor:`/`docs:`/`chore:`).

<!-- myco:agent-awareness -->

## Agent awareness

- Session start / any work discussion: run `myco status --prompt`
  before proposing next steps — know the board first.
- User describes new work → offer to capture it (`myco todo add`,
  `issue new`, `proposal new`). Don't wait to be asked.
- Picking up a task: `myco resume <ID>` is the briefing.
- Wrapping up: `myco handoff <ID>` anything resumable; keep statuses
  honest (claimed → in-progress → blocked/done).

<!-- /myco:agent-awareness -->

<!-- sturn:agent-awareness -->

## Sturn awareness

- **No PRs.** The pushed branch/commit is the deliverable; review
  streams continuously (sturn WAL), merge is a human/sturn call.
- sturn-wired repo (`sturn.toml` present): `sturn claim "<uri>"
  --ws <ws>` *before* edits — claims surface conflicts in ms.
- Ops via `sturn write|rename|delete`; `sturn snap` folds the ws
  into a real commit; `sturn push` publishes heads + refs/sturn/*.
- `sturn leave` on exit — drops the actor's claims.

<!-- /sturn:agent-awareness -->

<!-- kuda:agent-awareness -->

## Kuda awareness

- `.kuda/` cards are committed artifacts — read them before
  re-exploring (`kuda map`, `kuda card <mod>`); `kuda status`
  reports per-card drift vs HEAD.
- Code changes that invalidate a card → regenerate it (`kuda
  sync`) in the same change; cards ride the diff they describe.
- `.kuda/index.sqlite` is gitignored + rebuildable; the `*.yaml`
  cards are the checked-in source of truth.

<!-- /kuda:agent-awareness -->

## Project management

- All PM state lives in the `project/` worktree via `myco` —
  tasks, backlog, issues, proposals, ADRs, handoffs. Never track
  tasks, milestones, or status in README, docs, or code comments;
  point at myco instead. `project/TODO.md` is the source of truth.
