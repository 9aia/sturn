---
url: /sturn/reference/ops.md
---
# Ops

The event vocabulary appended to `refs/sturn/wal/<ws>`. JSON,
content-addressed (blob sha = event id), one commit per append.

## Tree ops

| Op | Fields | Effect at `snap` |
|---|---|---|
| `write` | `path`, `blob`, `intent?` | replace/create file |
| `rename` | `from`, `to` | move entry |
| `delete` | `path` | remove entry |

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
