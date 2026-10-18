# BACKLOG — unscheduled

- Signed ops (Ed25519) — attribution → authentication
- Review gates: `snap` refuses unreviewed ops (policy in sturn.toml)
- `libgrite-git` as alternative Wal impl (CRDT merge across
  diverged WAL refs)
- Federation: WAL refs synced via broker, not just git remote
- Semantic merge helpers (rename-aware) — ops carry the intent,
  a smarter merger could use it
- GitButler-style virtual branches on top of ws namespaces
- Web dashboard (ops firehose viz) — low value, high fun
- `sturn diff --live` — stream of in-flight ops vs main
- crates.io release loop (copy bupha's `scripts/release.sh` shape)
