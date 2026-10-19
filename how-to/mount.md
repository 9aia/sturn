# How-to: mount a workspace

`sturn mount` exposes a workspace tree as a real filesystem — the
interception layer: tools write normally, sturn records ops.

```
agent's write_file() ──► kernel ──► FUSE ──► SturnFs
                                          write() → buffer
                                          release → Op::Write → WAL
```

FUSE = *Filesystem in USErspace*: syscalls route to a user process
instead of a disk driver, so `sturn-fuse` decides what "a file"
means — ops, not bytes on disk.

## Use

```console
$ sturn mount devin /mnt/devin        # read-only view of the tree
$ sturn mount --rw devin /mnt/devin   # writes become WAL ops
```

Under `--rw`, plain POSIX writes — `echo`, editors, build tools,
agent `write_file` — land as `Write`/`Rename`/`Delete` ops with
`intent: "fuse"`. The agent can't bypass the write path: it *is* a
filesystem, the thing tools already use.

## Semantics

- **Buffer-then-commit**: `write()` buffers per open file; the op
  emits on `release` (close) or `fsync` — editors that fsync-per-
  save get an op at sync point, not just at close.
- **Mount-time view**: the tree is the ws tip at mount time —
  other agents' ops don't appear live (unmount/remount to see).
- **Dirs are implicit** in git trees — `mkdir`/`rmdir` are local
  only; a `Write` op into the dir is what persists.

```console
$ fusermount -u /mnt/devin
```
