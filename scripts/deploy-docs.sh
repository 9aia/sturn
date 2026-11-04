#!/usr/bin/env bash
# Build the VitePress site and publish .vitepress/dist to the `gh-pages`
# branch. Runs identically by hand (from the docs/ worktree) and under
# bupha — the watcher runs `bash scripts/deploy-docs.sh` in a detached
# worktree of the pushed docs sha, so cwd is the docs checkout root and
# `..` is NOT the main repo. Cargo work happens in STURN_REPO (the real
# checkout).
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
REPO="${STURN_REPO:-$HOME/GitHub/9aia/sturn}"

# The bupha watcher unit runs with the minimal systemd PATH
# (/usr/local/bin:/usr/bin) whenever it starts before the session env is
# imported — pull the user tool dirs in ourselves so bun/cargo resolve
# like they do in a login shell.
export PATH="$HOME/.bun/bin:$HOME/.cargo/bin:$HOME/.local/bin:$PATH"

# Isolated target dir — a cargo doc run sharing the default `target/`
# with agent/interactive builds is stale-rmeta bait. `target/gate`
# stays under the gitignored `target/`.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$REPO/target/gate}"

echo "==> bun install"
bun install

echo "==> rustdoc → public/api (cargo doc in $REPO)"
mkdir -p public/api && rm -rf public/api/*
(cd "$REPO" && cargo doc --workspace --no-deps)
cp -r "$CARGO_TARGET_DIR/doc/"* public/api/

echo "==> vitepress build"
bunx vitepress build

# Branch deploys still get a Jekyll pass unless .nojekyll is present.
touch .vitepress/dist/.nojekyll

echo "==> deploy .vitepress/dist → gh-pages"
bunx gh-pages -d .vitepress/dist -b gh-pages \
  -r git@github.com:9aia/sturn.git \
  -m "docs: deploy $(git rev-parse --short HEAD)"

echo "done — https://9aia.com/sturn/ serves origin/gh-pages"
