# Changelog

## [1.0.0] - 2026-11-24

### Added

- scaffold — WAL in git refs, claims, virtual workspaces, snap (`f5dafb8`)
- rename/delete surfaces + overlay-aware snap (dogfooded) (`a2fb761`)
- sturn-odb — direct loose-object I/O, ~4x faster WAL appends (`4dc95b1`)

### Changed

- **vcs:** consume lasius-vcs — drop sturn-vcs trait crate (`adae1d7`)
- de-concretize sturn-cli — dyn ports, Vcs::sync(remote), Vcs::resolve (`59ba77b`)
- **vcs:** move git plumbing onto the lasius-vcs port (P-049) (`0c569ff`)
