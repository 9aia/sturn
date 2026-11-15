# Crate map

Ports (traits only) and impls — new backends are new impl crates,
never core changes.

<!-- crate-map -->
| crate | layer | purpose |
|---|---|---|
| `sturn-ask` | domain | pending-response asks: fold ops into open questions and verdicts |
| `sturn-claim` | domain | advisory claims: fold ops into live locks, detect overlap |
| `sturn-core` | domain | domain types: op/event schema, sturn.toml config |
| `sturn-bus` | port | Bus port: topic pub/sub over the WAL (a cache, not authority) |
| `sturn-secret` | port | Secrets port: named-secret resolution |
| `sturn-service` | port | Service port: OS service lifecycle |
| `sturn-wal` | port | Wal port: append-only op log inside git refs |
| `sturn-ws` | port | Ws port: virtual workspaces over the git ODB |
| `sturn-bus-redb` | adapter | Bus impl: redb-backed topic store with cursors |
| `sturn-config` | adapter | ConfigSource port + scarab-config-file impl: sturn.{yaml,yml,toml} discovery |
| `sturn-dotenv` | adapter | Secrets impl: KEY=VAL file, chmod 600 |
| `sturn-fuse` | adapter | FUSE impl: mount a ws tree read-only for POSIX tools |
| `sturn-git` | adapter | Vcs impl via the git binary + WAL-&gt;commit snap |
| `sturn-keyring` | adapter | Secrets impl: secret-tool (libsecret) |
| `sturn-odb` | adapter | Direct git ODB access: loose objects, refs, trees — no process spawns |
| `sturn-systemd` | adapter | Service impl + deploy helpers: systemd user units |
| `sturn-wal-git` | adapter | Wal impl: ops as commits on refs/sturn/wal/* via git plumbing |
| `sturn-ws-git` | adapter | Ws impl: read/list/materialize via git plumbing |
| `sturn` | engine | sturn — ephemeral coordination engine: claims/ops/topics over VCS state (engine lib + compat bin) |
| `sturn-driver` | driver | tamias-driver-sturn — pavo view driver over a repo's sturn coordination state plus engine ops dispatching to the sturn lib |

Rustdoc: `/api/sturn_core/` etc. after `bun run docs:api`.
