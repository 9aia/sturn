---
url: /sturn/reference/crate-api.md
---
# Crate map

Ports (traits only) and impls — new backends are new impl crates,
never core changes.

| Crate | Role |
|---|---|
| `sturn-core` | `Event`/`Op`/`Verdict`, `Config`, topic names |
| `sturn-wal` → `sturn-wal-git` | append-only op log in git refs (CAS) |
| `sturn-bus` → `sturn-bus-redb` | topic pub/sub cache over the WAL |
| `sturn-ws` → `sturn-ws-git` | virtual workspaces over the ODB |
| `sturn-vcs` → `sturn-git` | remote sync/push; `snap_commit` bridge |
| `sturn-claim` | pure claim logic: fold → live locks → conflicts |
| `sturn-mcp` | the agent tool contract (M3 wires the server) |
| `sturn-config` → `sturn-config-toml` | config source |
| `sturn-secret` → `sturn-keyring`/`sturn-dotenv` | secret resolution |
| `sturn-service` → `sturn-systemd` | daemon lifecycle |
| `sturn-cli` | `sturn` binary — the composition root |

Rustdoc: `/api/sturn_core/` etc. after `bun run docs:api`.
