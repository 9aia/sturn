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

## Envelope

```json
{"id":"<blob sha>","ws":"agent-7","actor":"agent-7","ts":1792292994,
 "op":{"op":"write","path":"src/lib.rs","blob":"<sha>","intent":"…"}}
```
