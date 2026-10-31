+++
id = "I-002"
title = "FUSE mount tests flaky in sandboxed envs"
status = "open"
created = "2026-10-30"
priority = "medium"
+++

# FUSE mount tests flaky in sandboxed envs

Status: **open** · Created: 2026-10-30

## Description

FUSE-mount tests hang or flake in sandboxed environments where
/dev/fuse exists but mounts are unreliable:

- crates/sturn-cli/tests/e2e.rs — mount_real_and_missing_ws (hangs on mount)
- crates/sturn-fuse/tests/fuse.rs — read_error_surfaces_eio, read_only_mount_serves_tree

Observed: same test passes/fails randomly on a clean tree — environmental,
not related to the asks change. The e2e skip gate only checks /dev/fuse
exists, not that mounts actually work.

## Checklist

- [ ] Strengthen the mount-capability probe: try a real mount before
  running FUSE tests, or add a bounded wait_mounted timeout that skips
  instead of hanging
- [ ] Consider STURN_SKIP_FUSE env guard for CI/sandboxed runs

## Resolution

Open — tests are correct on a real FUSE host; the skip mechanism is the gap.
