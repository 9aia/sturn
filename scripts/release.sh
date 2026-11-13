#!/usr/bin/env bash
# sturn release — thin shim over tarsi-serve (the org release plane,
# project ADR-0005; repo config: ./tarsi.toml). One call runs
# plan → changelog → bump → tag → push; publish targets are off
# (ADR-0004: crates.io is tamias-only, GitHub Releases unused).
# Operator-run on main — `tarsi-serve release` is dry-run by
# default; --yes is the explicit opt-in.
#
# crates.io publish is GONE: the old per-crate `cargo publish`
# loop (with its hand-maintained, drift-prone publish-order list)
# and the `dist build` + `gh release upload` binary path are
# retired. The `sturn` command ships as the tamias pack in
# `libraries/sturn` (`tamias install --allow-drivers
# ./libraries/sturn`); `scripts/pack.sh` stages the driver payload.
#
# Pack-ref sync (`[release.sync_refs]` in tarsi.toml) rewrites the
# tamias pack manifest version inside the release commit, so the
# tagged tree carries consistent versions. The pack publish itself
# stays manual (`tamias publish libraries/sturn`).
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
# worktree-local env (gitignored); the service env file is canonical
[ -f ./.env ] && { set -a; source ./.env; set +a; }

tarsi-serve release --dir . --yes

echo "released: see tags sturn-*-v<ver>"
echo "next: scripts/pack.sh && tamias publish libraries/sturn  # ship 9aia:sturn@<ver> to the org registry"
