# Changelog

## [1.0.0] - 2026-11-24

### Added

- presence, compaction, signed ops, bupha bridge, FUSE mount (`805a938`)
- asks — pending-response ops as sturn's fourth primitive (`47181d3`)
- FUSE write-back, bus poll range-scan, mount --rw (`32a31bd`)
- **fuse:** fsync (op-at-sync-point), flush, mkdir, rmdir — editors that fsync-per-save no longer ENOSYS; verified live: echo→write op, mv→rename op in WAL (`241b1f2`)

### Changed

- **vcs:** consume lasius-vcs — drop sturn-vcs trait crate (`adae1d7`)

### Fixed

- **fuse:** keep inode maps in sync on rename/unlink — cross-dir mv no longer ENOENT (`17d814f`)
