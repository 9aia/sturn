# Changelog

## [1.0.0] - 2026-11-24

### Added

- scaffold — WAL in git refs, claims, virtual workspaces, snap (`f5dafb8`)
- M3 — MCP stdio server + overlay-aware reads (`9caf47b`)
- claims accept typed resource URIs — fs:/dev:/net:/sess:/cap: + sturn check (`8420a46`)
- presence, compaction, signed ops, bupha bridge, FUSE mount (`805a938`)
- review gate — snap refuses unreviewed tree ops (dogfooded) (`6d1a7c6`)
- **hooks:** [hooks] post_op — detached commands after ref-moving verbs (`4d1c005`)
- rename-aware semantic merge for multi-ws snap (`4c60cb3`)
- asks — pending-response ops as sturn's fourth primitive (`47181d3`)
- review gates, bus seq index, multi-repo daemon (`1a17bb1`)

### Changed

- **vcs:** consume lasius-vcs — drop sturn-vcs trait crate (`adae1d7`)
- **config:** scarab-config-file discovery — sturn.{yaml,yml,toml} (P-017) (`0823005`)

### Fixed

- resolve --ws . to the repo name, reject invalid ws names early (`ba7e3d4`)
