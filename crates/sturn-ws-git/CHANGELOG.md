# Changelog

## [1.0.0] - 2026-11-24

### Added

- scaffold — WAL in git refs, claims, virtual workspaces, snap (`f5dafb8`)
- M3 — MCP stdio server + overlay-aware reads (`9caf47b`)
- M2 — delta fan-out, push subscribers, real glob claims (`7926283`)
- M4 — overlay-aware hydration + `sturn exec` (`53adfaa`)
- sturn-odb — direct loose-object I/O, ~4x faster WAL appends (`4dc95b1`)

### Changed

- **vcs:** consume lasius-vcs — drop sturn-vcs trait crate (`adae1d7`)
- **vcs:** move git plumbing onto the lasius-vcs port (P-049) (`0c569ff`)
