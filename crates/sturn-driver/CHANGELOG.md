# Changelog

## [1.0.0] - 2026-11-24

### Added

- **driver:** tamias-driver-sturn — pavo views for claims/ops/topics (`ef4f5da`)
- **BREAKING** ops manifest is the agent surface — drop sturn-mcp (`8822000`)
- **pack:** tamias bins[] pack — sturn rides the driver shim (P-021/T-012) (`7a77a67`)
- engine-as-lib + tamias driver ops pattern (kuda/myco parity) (`52b7220`)

### Changed

- **sturn-driver:** adopt pavo-driver-kit (`bf3fc6b`)
- **vcs:** consume lasius-vcs — drop sturn-vcs trait crate (`adae1d7`)
- **vcs:** move git plumbing onto the lasius-vcs port (P-049) (`0c569ff`)
- **config:** scarab-config-file discovery — sturn.{yaml,yml,toml} (P-017) (`0823005`)

### Fixed

- **driver:** handle pavo Request::Invoke — bump pavo lock to ca4079d (`5d979c8`)
