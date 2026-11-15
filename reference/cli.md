# Command-Line Help for `sturn`

This document contains the help content for the `sturn` command-line program.

Evented coordination over the git object store — agents murmurate, not merge

**Command Overview:**

* [`sturn`↴](#sturn)
* [`sturn init`↴](#sturn-init)
* [`sturn ws`↴](#sturn-ws)
* [`sturn ws new`↴](#sturn-ws-new)
* [`sturn ws ls`↴](#sturn-ws-ls)
* [`sturn ws read`↴](#sturn-ws-read)
* [`sturn ws paths`↴](#sturn-ws-paths)
* [`sturn ws grep`↴](#sturn-ws-grep)
* [`sturn ws hydrate`↴](#sturn-ws-hydrate)
* [`sturn claim`↴](#sturn-claim)
* [`sturn release`↴](#sturn-release)
* [`sturn write`↴](#sturn-write)
* [`sturn rename`↴](#sturn-rename)
* [`sturn delete`↴](#sturn-delete)
* [`sturn intent`↴](#sturn-intent)
* [`sturn compact`↴](#sturn-compact)
* [`sturn log`↴](#sturn-log)
* [`sturn diff`↴](#sturn-diff)
* [`sturn snap`↴](#sturn-snap)
* [`sturn review`↴](#sturn-review)
* [`sturn push`↴](#sturn-push)
* [`sturn daemon`↴](#sturn-daemon)
* [`sturn sub`↴](#sturn-sub)
* [`sturn join`↴](#sturn-join)
* [`sturn leave`↴](#sturn-leave)
* [`sturn verify`↴](#sturn-verify)
* [`sturn claims`↴](#sturn-claims)
* [`sturn check`↴](#sturn-check)
* [`sturn ask`↴](#sturn-ask)
* [`sturn asks`↴](#sturn-asks)
* [`sturn answer`↴](#sturn-answer)
* [`sturn cancel`↴](#sturn-cancel)
* [`sturn wait`↴](#sturn-wait)
* [`sturn bridge`↴](#sturn-bridge)
* [`sturn exec`↴](#sturn-exec)
* [`sturn mount`↴](#sturn-mount)
* [`sturn deploy`↴](#sturn-deploy)
* [`sturn deploy service`↴](#sturn-deploy-service)
* [`sturn deploy env`↴](#sturn-deploy-env)
* [`sturn destroy`↴](#sturn-destroy)
* [`sturn destroy service`↴](#sturn-destroy-service)
* [`sturn service`↴](#sturn-service)
* [`sturn service enable`↴](#sturn-service-enable)
* [`sturn service disable`↴](#sturn-service-disable)
* [`sturn service status`↴](#sturn-service-status)
* [`sturn service restart`↴](#sturn-service-restart)
* [`sturn service logs`↴](#sturn-service-logs)

## `sturn`

Evented coordination over the git object store — agents murmurate, not merge

**Usage:** `sturn [OPTIONS] <COMMAND>`

###### **Commands:**

* `init` — Wire a repo for sturn — writes sturn.toml if absent
* `ws` — Workspace namespaces — virtual checkouts that cost a pointer
* `claim` — Advisory lock on a resource URI — published *before* edits
* `release` — Drop a claim — same URI form `claim` takes
* `write` — Append a write op — --content, --file, or stdin becomes an ODB blob
* `rename` — Append a rename op — intent survives where git loses it
* `delete` — Append a delete op
* `intent` — Record why work exists — provenance as data
* `compact` — Drop pre-snapshot WAL history (archived, not lost)
* `log` — Replay the WAL — ops in order, JSONL. `--topic`/`--limit` filter the stream; `--wait-ms` long-polls (bounded) for new events before printing — the daemon-free `subs` pull
* `diff` — In-flight ops per workspace — logged since the last snap, not yet landed. `--live` keeps streaming as new ops arrive
* `snap` — Fold ws ops into a real commit on the branch. Multiple ws = merged into ONE commit — rename-aware (writes follow renames); semantic conflicts warn on stderr, later ops win same-path
* `review` — Approve/reject an event — continuous review
* `push` — Push heads + refs/sturn/* — coordination rides the remote
* `daemon` — WAL→topic fan-out loop. The bus is a cache — ops still sync through git when it's down
* `sub` — Stream a topic live — ops|claims|review|presence, JSONL
* `join` — Announce presence — `leave` drops all the actor's claims
* `leave` — Leave a ws — sweeps the actor's claims on it
* `verify` — Verify event signatures against allowed_signers
* `claims` — Live claims across every workspace
* `check` — List live claims conflicting with a URI — consult before acting on a resource. Exits 1 when the URI is contested
* `ask` — Open a pending-response ask — the printed event id is the ask handle that `answer`/`cancel`/`wait` take
* `asks` — Pending asks across every workspace — open by default
* `answer` — Answer an open ask — the verdict rides your ws's WAL
* `cancel` — Withdraw an open ask
* `wait` — Block until an ask resolves — prints the result as JSON. Exit: 0 answered, 2 cancelled, 3 expired, 4 timeout. `--timeout 0` (the default) waits until resolved — unbounded; a bare `cli.wait` op can park a request forever, pass `--timeout` for a bounded wait
* `bridge` — Bridge snapshot events to a bupha webhook — GitHub-shaped, HMAC-signed. Bupha runs CI on snaps with zero bupha changes
* `exec` — Hydrate a ws to tmpfs and run a command inside — compilers need POSIX; agents don't. Exits with the command's status
* `mount` — Mount a ws tree via FUSE — POSIX for tools/GUIs. --rw: edits flow back as Write/Delete/Rename ops
* `deploy` — Deploy the daemon — systemd user unit + env skeleton
* `destroy` — Teardown counterparts of `deploy`
* `service` — Toggle the daemon unit without removing it — free the box when fan-out isn't needed, `enable` brings it back

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help
* `-V, --version` — Print version

## `sturn init`

Wire a repo for sturn — writes sturn.toml if absent

**Usage:** `sturn init [OPTIONS]`

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn ws`

Workspace namespaces — virtual checkouts that cost a pointer

**Usage:** `sturn ws [OPTIONS] <COMMAND>`

###### **Commands:**

* `new` — Create a workspace — zero materialization, just a ref
* `ls` — List workspaces
* `read` — Read a file from a ws — the live view: tip + uncommitted ops, same fold `hydrate` materializes
* `paths` — List paths in a ws tree (tip + pending ops), optionally glob-filtered — `**` lists everything
* `grep` — Substring search across a ws tree (tip + pending ops) — `path:line: text` per hit. `--path` restricts to a glob
* `hydrate` — Materialize a ws tree into a dir — tip + uncommitted ops. --path filters to a subset (repeatable); empty = whole tree

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn ws new`

Create a workspace — zero materialization, just a ref

**Usage:** `sturn ws new [OPTIONS] <NAME>`

###### **Arguments:**

* `<NAME>`

###### **Options:**

* `--base <BASE>` — Base commit/branch (default: HEAD)
* `--repo <REPO>` — Repo dir (default: cwd)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn ws ls`

List workspaces

**Usage:** `sturn ws ls [OPTIONS]`

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn ws read`

Read a file from a ws — the live view: tip + uncommitted ops, same fold `hydrate` materializes

**Usage:** `sturn ws read [OPTIONS] <NAME> <PATH>`

###### **Arguments:**

* `<NAME>`
* `<PATH>`

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn ws paths`

List paths in a ws tree (tip + pending ops), optionally glob-filtered — `**` lists everything

**Usage:** `sturn ws paths [OPTIONS] <NAME> [PATTERN]`

###### **Arguments:**

* `<NAME>`
* `[PATTERN]` — Glob filter — same grammar claims use

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn ws grep`

Substring search across a ws tree (tip + pending ops) — `path:line: text` per hit. `--path` restricts to a glob

**Usage:** `sturn ws grep [OPTIONS] <NAME> <PATTERN>`

###### **Arguments:**

* `<NAME>`
* `<PATTERN>` — Substring to find

###### **Options:**

* `--path <PATH>` — Only search paths matching this glob
* `--repo <REPO>` — Repo dir (default: cwd)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn ws hydrate`

Materialize a ws tree into a dir — tip + uncommitted ops. --path filters to a subset (repeatable); empty = whole tree

**Usage:** `sturn ws hydrate [OPTIONS] <NAME> <DIR>`

###### **Arguments:**

* `<NAME>`
* `<DIR>`

###### **Options:**

* `--path <PATHS>`
* `--repo <REPO>` — Repo dir (default: cwd)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `--writeback` — After materializing, diff the dir back into ops now
* `-h, --help` — Print help

## `sturn claim`

Advisory lock on a resource URI — published *before* edits

**Usage:** `sturn claim [OPTIONS] --ws <WS> <URI>`

###### **Arguments:**

* `<URI>` — Resource URI or path glob — "fs:src/**", "dev:display0", "net:13000", "sess:T-012"; a bare glob means "fs:"

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--ws <WS>` — Workspace claiming it
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `--ttl <TTL>` — Seconds the claim lives (auto-expiry, no zombie locks) [default: 3600]
* `--force` — Take the claim even if it conflicts with a live one
* `-h, --help` — Print help

## `sturn release`

Drop a claim — same URI form `claim` takes

**Usage:** `sturn release [OPTIONS] --ws <WS> <URI>`

###### **Arguments:**

* `<URI>`

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--ws <WS>`
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn write`

Append a write op — --content, --file, or stdin becomes an ODB blob

**Usage:** `sturn write [OPTIONS] --ws <WS> <PATH>`

###### **Arguments:**

* `<PATH>` — Path inside the tree

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--ws <WS>`
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `--file <FILE>`
* `--content <CONTENT>` — Inline content — the wire-friendly form (ops pass it as a param; no stdin needed)
* `--intent <INTENT>` — Why this edit exists — attached to the op
* `-h, --help` — Print help

## `sturn rename`

Append a rename op — intent survives where git loses it

**Usage:** `sturn rename [OPTIONS] --ws <WS> <FROM> <TO>`

###### **Arguments:**

* `<FROM>`
* `<TO>`

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--ws <WS>`
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn delete`

Append a delete op

**Usage:** `sturn delete [OPTIONS] --ws <WS> <PATH>`

###### **Arguments:**

* `<PATH>`

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--ws <WS>`
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn intent`

Record why work exists — provenance as data

**Usage:** `sturn intent [OPTIONS] --ws <WS> <TEXT>`

###### **Arguments:**

* `<TEXT>`

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--ws <WS>`
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn compact`

Drop pre-snapshot WAL history (archived, not lost)

**Usage:** `sturn compact [OPTIONS]`

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--ws <WS>` — One workspace, or all when omitted
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn log`

Replay the WAL — ops in order, JSONL. `--topic`/`--limit` filter the stream; `--wait-ms` long-polls (bounded) for new events before printing — the daemon-free `subs` pull

**Usage:** `sturn log [OPTIONS]`

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--ws <WS>` — One workspace, or all when omitted
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `--topic <TOPIC>` — Restrict to one topic: ops|claims|review|presence|asks
* `--limit <LIMIT>` — Keep only the last N matching events
* `--wait-ms <WAIT_MS>` — Wait up to N ms for new events before printing (0/omitted = snapshot now)
* `-h, --help` — Print help

## `sturn diff`

In-flight ops per workspace — logged since the last snap, not yet landed. `--live` keeps streaming as new ops arrive

**Usage:** `sturn diff [OPTIONS]`

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--ws <WS>` — One workspace, or all when omitted
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `--json` — Emit the raw events as JSONL (same shape `log` produces)
* `--live` — Keep streaming new ops — WAL poll, works daemon-free
* `-h, --help` — Print help

## `sturn snap`

Fold ws ops into a real commit on the branch. Multiple ws = merged into ONE commit — rename-aware (writes follow renames); semantic conflicts warn on stderr, later ops win same-path

**Usage:** `sturn snap [OPTIONS] --message <MESSAGE> <WS>...`

###### **Arguments:**

* `<WS>...`

###### **Options:**

* `-m, --message <MESSAGE>`
* `--repo <REPO>` — Repo dir (default: cwd)
* `--branch <BRANCH>` — Branch to update (default: repo.branch / main)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn review`

Approve/reject an event — continuous review

**Usage:** `sturn review [OPTIONS] --ws <WS> <EVENT> <VERDICT>`

###### **Arguments:**

* `<EVENT>` — Event id (blob sha) to judge
* `<VERDICT>` — approve | reject

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--ws <WS>`
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn push`

Push heads + refs/sturn/* — coordination rides the remote

**Usage:** `sturn push [OPTIONS]`

###### **Options:**

* `--remote <REMOTE>` — [default: origin]
* `--repo <REPO>` — Repo dir (default: cwd)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn daemon`

WAL→topic fan-out loop. The bus is a cache — ops still sync through git when it's down

**Usage:** `sturn daemon [OPTIONS]`

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn sub`

Stream a topic live — ops|claims|review|presence, JSONL

**Usage:** `sturn sub [OPTIONS] <TOPIC>`

###### **Arguments:**

* `<TOPIC>` — Topic suffix under `repo.<id>.`

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn join`

Announce presence — `leave` drops all the actor's claims

**Usage:** `sturn join [OPTIONS] <WS>`

###### **Arguments:**

* `<WS>`

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn leave`

Leave a ws — sweeps the actor's claims on it

**Usage:** `sturn leave [OPTIONS] <WS>`

###### **Arguments:**

* `<WS>`

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn verify`

Verify event signatures against allowed_signers

**Usage:** `sturn verify [OPTIONS]`

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--ws <WS>`
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn claims`

Live claims across every workspace

**Usage:** `sturn claims [OPTIONS]`

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn check`

List live claims conflicting with a URI — consult before acting on a resource. Exits 1 when the URI is contested

**Usage:** `sturn check [OPTIONS] <URI>`

###### **Arguments:**

* `<URI>` — Resource URI or path glob — same form `claim` takes

###### **Options:**

* `--fence <FENCE>` — Staleness probe: report only conflicting claims with a fencing token above N — pass the fence `claim` printed to ask "has my claim been superseded?"
* `--repo <REPO>` — Repo dir (default: cwd)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `--watch` — Keep watching — stream conflict-set changes via WAL poll. Works daemon-free; combines with --fence
* `-h, --help` — Print help

## `sturn ask`

Open a pending-response ask — the printed event id is the ask handle that `answer`/`cancel`/`wait` take

**Usage:** `sturn ask [OPTIONS] --ws <WS> <SUBJECT>`

###### **Arguments:**

* `<SUBJECT>` — What is being asked

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--ws <WS>` — Workspace asking
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `--target <TARGET>` — Who may answer — free-form spec: `actor:<name>`, `human:<name>`, `node:<host>`, `claim-holder:<glob>`, `any` [default: any]
* `--option <OPTIONS>` — Offered verdict — repeatable, advisory only
* `--ttl <TTL>` — Seconds until the ask expires. 0 = never [default: 3600]
* `-h, --help` — Print help

## `sturn asks`

Pending asks across every workspace — open by default

**Usage:** `sturn asks [OPTIONS]`

###### **Options:**

* `--all` — Include resolved asks (answered/cancelled/expired)
* `--repo <REPO>` — Repo dir (default: cwd)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `--json` — Emit a JSON array instead of text lines
* `-h, --help` — Print help

## `sturn answer`

Answer an open ask — the verdict rides your ws's WAL

**Usage:** `sturn answer [OPTIONS] --ws <WS> <ASK> <VERDICT>`

###### **Arguments:**

* `<ASK>` — Ask event id
* `<VERDICT>`

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--ws <WS>` — Workspace answering
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn cancel`

Withdraw an open ask

**Usage:** `sturn cancel [OPTIONS] --ws <WS> <ASK>`

###### **Arguments:**

* `<ASK>` — Ask event id

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--ws <WS>`
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn wait`

Block until an ask resolves — prints the result as JSON. Exit: 0 answered, 2 cancelled, 3 expired, 4 timeout. `--timeout 0` (the default) waits until resolved — unbounded; a bare `cli.wait` op can park a request forever, pass `--timeout` for a bounded wait

**Usage:** `sturn wait [OPTIONS] <ASK>`

###### **Arguments:**

* `<ASK>` — Ask event id

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--timeout <TIMEOUT>` — Max seconds to wait. 0 = until resolved (unbounded — the default) [default: 0]
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `--poll <POLL>` — Poll interval, ms [default: 100]
* `-h, --help` — Print help

## `sturn bridge`

Bridge snapshot events to a bupha webhook — GitHub-shaped, HMAC-signed. Bupha runs CI on snaps with zero bupha changes

**Usage:** `sturn bridge [OPTIONS] --to <TO>`

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--to <TO>` — bupha listener URL (e.g. http://127.0.0.1:29173/)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `--secret-env <SECRET_ENV>` — Env var naming the webhook secret (HMAC-SHA256) [default: BUPHA_WEBHOOK_SECRET]
* `-h, --help` — Print help

## `sturn exec`

Hydrate a ws to tmpfs and run a command inside — compilers need POSIX; agents don't. Exits with the command's status

**Usage:** `sturn exec [OPTIONS] --ws <WS> <CMD>...`

###### **Arguments:**

* `<CMD>...` — Command + args, after `--`

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--ws <WS>` — Workspace to hydrate
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `--keep` — Keep the hydrated dir after exit
* `--writeback` — Write filesystem edits back as ops on the ws
* `-h, --help` — Print help

## `sturn mount`

Mount a ws tree via FUSE — POSIX for tools/GUIs. --rw: edits flow back as Write/Delete/Rename ops

**Usage:** `sturn mount [OPTIONS] <WS> <DIR>`

###### **Arguments:**

* `<WS>`
* `<DIR>`

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--rw`
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn deploy`

Deploy the daemon — systemd user unit + env skeleton

**Usage:** `sturn deploy [OPTIONS] <COMMAND>`

###### **Commands:**

* `service` — Write + enable the systemd user unit for `sturn daemon`
* `env` — Seed ~/.config/sturn/env (0600, names only)

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn deploy service`

Write + enable the systemd user unit for `sturn daemon`

**Usage:** `sturn deploy service [OPTIONS]`

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn deploy env`

Seed ~/.config/sturn/env (0600, names only)

**Usage:** `sturn deploy env [OPTIONS]`

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn destroy`

Teardown counterparts of `deploy`

**Usage:** `sturn destroy [OPTIONS] <COMMAND>`

###### **Commands:**

* `service` — Disable + remove the systemd user unit

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn destroy service`

Disable + remove the systemd user unit

**Usage:** `sturn destroy service [OPTIONS]`

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn service`

Toggle the daemon unit without removing it — free the box when fan-out isn't needed, `enable` brings it back

**Usage:** `sturn service [OPTIONS] <COMMAND>`

###### **Commands:**

* `enable` — Enable + start the daemon unit now and at login
* `disable` — Stop the daemon now; stays stopped across logins until `service enable` (the unit file is kept)
* `status` — Whether the daemon unit is enabled and running
* `restart` — Bounce the daemon unit in place
* `logs` — Recent journal lines — `-f` follows the stream live

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn service enable`

Enable + start the daemon unit now and at login

**Usage:** `sturn service enable [OPTIONS]`

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn service disable`

Stop the daemon now; stays stopped across logins until `service enable` (the unit file is kept)

**Usage:** `sturn service disable [OPTIONS]`

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn service status`

Whether the daemon unit is enabled and running

**Usage:** `sturn service status [OPTIONS]`

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn service restart`

Bounce the daemon unit in place

**Usage:** `sturn service restart [OPTIONS]`

###### **Options:**

* `--repo <REPO>` — Repo dir (default: cwd)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help

## `sturn service logs`

Recent journal lines — `-f` follows the stream live

**Usage:** `sturn service logs [OPTIONS]`

###### **Options:**

* `-f, --f` — Follow the log stream (journalctl -f)
* `--repo <REPO>` — Repo dir (default: cwd)
* `--config <CONFIG>` — Config path (default: $STURN_CONFIG → ./sturn.toml → ~/.config/sturn/)
* `-h, --help` — Print help
