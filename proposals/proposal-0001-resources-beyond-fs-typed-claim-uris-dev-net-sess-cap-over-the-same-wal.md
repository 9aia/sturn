+++
id = "P-001"
title = "resources beyond fs — typed claim URIs (dev/net/sess/cap) over the same WAL"
status = "open"
created = "2026-11-03"
owner = "human:luis"
priority = "medium"
+++

Status: **proposal v1**. Generalizes claims from worktree globs to
typed resources. Motivation surfaced in `9aia/project` P-001 (pavo):
a capture device (display/GPU) is contested but isn't a path —
`sturn claim "display:0"` has no semantics today.

## Problem

Claims are implicitly `fs:`-scoped. Contested non-file resources —
displays, GPUs, ports, sessions, roles — can't be coordinated;
every engine reinvents its own busy-lock (or doesn't, and races).

## Locked decisions

- **Resources are typed URIs** — `scheme:path`:
  `fs:src/**/*.rs` · `dev:display0` · `net:13000` ·
  `sess:T-012` · `cap:release-cutter`.
- **Conflict = scheme-typed overlap** — `fs:` keeps glob
  intersection; opaque schemes conflict on exact/prefix match.
  v1 ships `fs:` + opaque-string schemes only — device claims work
  with zero new semantics.
- **Advisory stays advisory** — sturn surfaces, never blocks. The
  resource's owner consults `sturn check <uri>` before acting
  (pavo before grabbing a display, sepia before driving a session).
- **TTL + fencing token** — claims already expire; add a monotonic
  epoch so a holder can detect a stale usurper.
- **Substrate unchanged** — WAL, `refs/sturn/*` replication,
  `snap`/`push`/`leave` lifecycle. Entries carry resource URIs
  instead of bare globs; `fs:` glob claims are the same wire
  shape under an explicit scheme.

## Model

Unify the verbs: claims, ops, topics are three semantics over
ephemeral, namespaced, TTL'd atoms — conflict-surfacing /
mutate / notify. VCS claims become the first shipped scheme, not
the only one.

## What sturn becomes

The factory's **ephemeral coordination plane**: advisory claims
over namespaced resources + topics, WAL-backed, no daemon.

Boundary: sturn = mechanism — who asserts what, right now.
peregrin = policy — who *wins* a contested claim (takeover,
queueing). Same mechanism/policy split as sepia/peregrin.

## Phases

- **v1** — resource URI in the claim record; `sturn check <uri>`;
  opaque-scheme conflict = equality/prefix.
- **Later** — fencing tokens; `watch`/subscribe on resource
  patterns; scheme plugins for non-file overlap (e.g. `net:` port
  ranges); owner-side consult helpers in pavo/sepia skills.
- **Never** — enforcement/blocking locks (kills the ephemeral
  model); arbitration (peregrin's); durable registry (myco's).
