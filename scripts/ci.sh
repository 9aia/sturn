#!/usr/bin/env bash
# ci-gate gate for main — detached worktree of the pushed sha via
# bupha. Rust floor + org docs contract (no xtask in this repo).
set -euo pipefail

cargo test --workspace

bash scripts/docs-check.sh
