---
url: /sturn/explanation/model.md
---
# The model — files, approval, agents

## Files are a projection, not the state

Git materializes a working tree because POSIX tools need paths.
Sturn keeps the tree *virtual*: `ws_read` serves blob contents
straight from the object database — no checkout, no index, no lock.

When something genuinely needs real files:

* `sturn exec --ws w -- cmd` — hydrate the ws view (tip +
  uncommitted ops) to tmpfs, run, discard
* `sturn mount w /dir --rw` — live FUSE filesystem; edits buffer
  per-open and become Write/Delete/Rename ops on release
* `--writeback` on `exec`/`hydrate` — dir diffs flow back as ops

Agents aren't filesystem-deprived — `ws_read/ws_write/ws_glob/
ws_grep` are the same file primitives harnesses already use; the
backend is virtualized, not the interaction.

## Approval without the PR cliff

Every op is content-addressed and streams through the WAL. A
reviewer emits `review <op-id> approve|reject` — itself a signed,
attributed op. With `[review] require = true`, `snap` refuses
unreviewed tree ops and names their ids — the gate exists, but
verdicts stream *per-op before commit time* instead of one giant
diff at the end. PRs become a packaging format, not the gate.

## Agents review agents

A verdict is just an op — any actor appends it. Intended topology:
fast writers produce ops → a senior agent or human watches `subs
ops|review` and emits verdicts → gated `snap` lands only approved
work. Humans spot-check the stream instead of serializing merges.
