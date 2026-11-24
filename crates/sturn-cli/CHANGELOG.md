# Changelog

## [1.0.0] - 2026-11-24

### Added

- scaffold — WAL in git refs, claims, virtual workspaces, snap (`f5dafb8`)
- sturn init converges AGENTS.md agent-awareness block (`df6d537`)
- hydrate --writeback (landed via live MCP session) (`ce7921e`)
- colocated-jj support — init hint + docs (`b97bd8c`)
- claim fencing tokens — check --fence staleness probe, --watch stream, net: ranges (`b86ac4c`)
- rename/delete surfaces + overlay-aware snap (dogfooded) (`a2fb761`)
- sturn diff — in-flight ops per ws, --live stream, --json (`a15c72d`)
- M3 — MCP stdio server + overlay-aware reads (`9caf47b`)
- **BREAKING** ops manifest is the agent surface — drop sturn-mcp (`8822000`)
- claims accept typed resource URIs — fs:/dev:/net:/sess:/cap: + sturn check (`8420a46`)
- presence, compaction, signed ops, bupha bridge, FUSE mount (`805a938`)
- M2 — delta fan-out, push subscribers, real glob claims (`7926283`)
- review gate — snap refuses unreviewed tree ops (dogfooded) (`6d1a7c6`)
- writeback exec, multi-ws merge snap, presence-on-connect (dogfooded) (`6a5c95a`)
- M4 — overlay-aware hydration + `sturn exec` (`53adfaa`)
- engine-as-lib + tamias driver ops pattern (kuda/myco parity) (`52b7220`)
- **hooks:** [hooks] post_op — detached commands after ref-moving verbs (`4d1c005`)
- rename-aware semantic merge for multi-ws snap (`4c60cb3`)
- asks — pending-response ops as sturn's fourth primitive (`47181d3`)
- FUSE write-back, bus poll range-scan, mount --rw (`32a31bd`)
- review gates, bus seq index, multi-repo daemon (`1a17bb1`)

### Changed

- **vcs:** consume lasius-vcs — drop sturn-vcs trait crate (`adae1d7`)
- retire sturn-service + sturn-systemd onto tamias-deploy-systemd (PL-025) (`6065222`)
- de-concretize sturn-cli — dyn ports, Vcs::sync(remote), Vcs::resolve (`59ba77b`)
- **config:** scarab-config-file discovery — sturn.{yaml,yml,toml} (P-017) (`0823005`)

### Fixed

- resolve --ws . to the repo name, reject invalid ws names early (`ba7e3d4`)
- **fuse:** keep inode maps in sync on rename/unlink — cross-dir mv no longer ENOENT (`17d814f`)
