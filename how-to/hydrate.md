# Hydrate a workspace

Agents read through the ODB; compilers need POSIX files. Hydrate a
workspace only when something must `exec`:

```sh
sturn ws hydrate agent-7 /tmp/ws-7   # git archive | tar — no index
cargo check --manifest-path /tmp/ws-7/Cargo.toml
```

- Hydration goes to tmpfs (`/dev/shm` or `/tmp`) — it is a
  projection, delete freely.
- M4 will add per-path hydration (only files an op touched) and a
  FUSE impl for humans who want a real mount.
