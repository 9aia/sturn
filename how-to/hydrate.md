# Hydrate a workspace

Agents read through the ODB; compilers need POSIX files. Hydration
materializes the ws *view* — tip commit **plus** uncommitted WAL
ops, matching what `ws_read` sees.

```sh
sturn ws hydrate agent-7 /tmp/ws-7            # whole view
sturn ws hydrate agent-7 /tmp/ws-7 --path src # subset
```

Or hydrate + run + clean in one step:

```sh
sturn exec --ws agent-7 -- cargo check        # tmpfs, gone after
sturn exec --ws agent-7 --keep -- cargo test  # keep the dir
```

- Target tmpfs (`/dev/shm`/`/tmp`) — it's a projection, delete freely.
- Exit status propagates: `sturn exec` fails iff the command does.
- Writes inside the hydrated dir flow back as ops only with
  `--writeback` (`sturn exec --ws <w> --writeback -- cmd` or
  `sturn ws hydrate <w> <dir> --writeback`) — a post-run dir diff
  becomes Write/Delete/Rename ops. Without it, ops remain the only
  write path.
- FUSE mounts for humans: `sturn mount <w> <dir> --rw` — see
  [mount](mount.md).
