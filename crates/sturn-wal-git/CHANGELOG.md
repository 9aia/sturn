# Changelog

## [1.0.0] - 2026-11-24

### Added

- scaffold — WAL in git refs, claims, virtual workspaces, snap (`f5dafb8`)
- presence, compaction, signed ops, bupha bridge, FUSE mount (`805a938`)
- M2 — delta fan-out, push subscribers, real glob claims (`7926283`)
- sturn-odb — direct loose-object I/O, ~4x faster WAL appends (`4dc95b1`)

### Changed

- **vcs:** consume lasius-vcs — drop sturn-vcs trait crate (`adae1d7`)
- **vcs:** move git plumbing onto the lasius-vcs port (P-049) (`0c569ff`)

### Fixed

- resolve --ws . to the repo name, reject invalid ws names early (`ba7e3d4`)
- **fuse:** keep inode maps in sync on rename/unlink — cross-dir mv no longer ENOENT (`17d814f`)
