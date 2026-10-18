# Security Policy

Report vulnerabilities privately to the maintainers via GitHub:
https://github.com/9aia/sturn/security/advisories/new

Do not open public issues for security reports.

## Scope

- Ops append to `refs/sturn/wal/<ws>` as git objects — they replicate
  over ordinary push/fetch, so the repo's existing transport auth
  (SSH keys, HTTPS tokens) is the whole auth story. There is no
  second gate.
- Events carry `actor` (from `$STURN_ACTOR`/`$USER`/`[actor].name`)
  as *attribution*, not authentication — do not build authorization
  on it until signed ops land (Ed25519, tracked in BACKLOG).
- Claims are advisory. `--force` exists; claim denial is a
  coordination hint, not a security boundary.
- Secrets resolve via env var → OS keyring (`secret-tool`) →
  `~/.config/sturn/env` (chmod 600); values never appear in config
  files.
- `snap` writes commits as the local git identity; signing comes
  from the repo's existing `commit.gpgsign`/`user.signingkey`.

## Notes

Event payloads are attacker-influenceable if anyone with push access
to the repo can write `refs/sturn/*` — treat WAL contents as
untrusted input (rejects unknown op variants; glob matchers are
non-executing). `sturn.toml` rejects unknown fields.

We respond to reports as quickly as we can; expect acknowledgement
within a few days.
