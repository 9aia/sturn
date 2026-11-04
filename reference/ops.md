# Ops

The event vocabulary appended to `refs/sturn/wal/<ws>`. JSON,
content-addressed (blob sha = event id), one commit per append.

## Tree ops

| Op | Fields | Effect at `snap` |
|---|---|---|
| `write` | `path`, `blob`, `intent?` | replace/create file |
| `rename` | `from`, `to` | move entry |
| `delete` | `path` | remove entry |

## Multi-workspace merge

`sturn snap a b …` folds several workspaces' pending ops into one
commit — rename-aware, not a bare path concat:

- a `write` onto a path *another* ws renamed away follows the
  rename to its destination (the modify-vs-rename case a 3-way
  merge resolves the same way), chained f→g→h and applied after
  the rename no matter the ws order;
- genuinely ambiguous shapes stay put and warn on stderr instead
  of picking a silent winner: a source renamed two ways
  (`rename f diverged`), a `delete` meeting a rename (either
  side), colliding destinations, and cross-ws same-path
  write/delete collisions.

## Coordination ops

| Op | Fields | Effect |
|---|---|---|
| `claim` | `glob`, `ttl_secs` | advisory lock; auto-expires |
| `release` | `glob` | drop a claim |
| `intent` | `text` | provenance — why the work exists |
| `review` | `event`, `verdict` | approve/reject an event id |
| `snapshot` | `message` | marker — ops before it are committed |
| `presence` | `status` | actor joined/left |
| `ask` | `subject`, `target`, `options?`, `ttl_secs` | pending *response* — open question; the event id is the ask handle |
| `answer` | `ask`, `verdict` | close an ask with a verdict |
| `cancel` | `ask` | withdraw an open ask |

## Claims

`claim.glob` carries a resource URI — `scheme:path`. `fs:` globs
(`src/**`, and a bare glob means `fs:`) or opaque resources
(`dev:display0`, `sess:T-012`, `cap:role`). `net:` paths may be
port ranges — `net:13000-13100` conflicts with `net:13050`;
non-numeric paths fall back to prefix matching.

Each live claim derives a **fencing token** at fold time — a
Lamport-style epoch: `1 + max` over earlier claims on overlapping
URIs, ordered by `ts` then WAL position. Never stored, and never
repeats for a resource — expired and released claims still count
as predecessors. `sturn claim` prints it; a holder passes it to
`sturn check <uri> --fence <n>` to ask "has anything outranked
me?" — the stale-usurper probe. It's advisory like the claim:
second-granularity `ts` plus concatenation order means
same-second cross-ws races resolve deterministically, not
temporally.

## Asks

Claims are pending *ownership*; asks are pending *answers*. An
`ask` opens under its event id and stays `open` until an `answer`
or `cancel` op lands — or `ttl_secs` runs out (`0` = never). The
question and its answer can live on different workspaces' WALs;
responders filter `target` (`actor:<n>`, `human:<n>`, `node:<h>`,
`claim-holder:<glob>`, `any`) to find asks meant for them.

## Envelope

```json
{"id":"<blob sha>","ws":"agent-7","actor":"agent-7","ts":1792292994,
 "op":{"op":"write","path":"src/lib.rs","blob":"<sha>","intent":"…"}}
```
