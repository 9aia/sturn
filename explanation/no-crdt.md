---
url: /sturn/explanation/no-crdt.md
---
# Why no CRDT for code

CRDTs are right for the *log*, wrong for the *tree*.

**For the WAL:** when replicas append ops and diverge, a CRDT-style
merge (grow-only set + causal order) converges the log with no
coordination — same events, same state. That part we keep.

**For the tree:** a CRDT auto-merge produces a *syntactically*
merged file, not a semantically correct one. Two agents inserting
different `main()` bodies converge to a file that compiles only by
luck. Code needs decisions, not convergence.

So sturn splits the difference:

* The **log** merges deterministically (CRDT semantics; a
  `libgrite-git` impl is on the backlog).
* The **tree** is produced by `snap` — a deterministic reduction
  over ops on a base. When ops collide semantically, that's a
  decision for a reviewer, not a merge algorithm.
* **Claims** make most collisions never happen — prevention beats
  resolution.
