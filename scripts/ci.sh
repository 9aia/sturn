#!/usr/bin/env bash
# ci-gate gate for main — detached worktree of the pushed sha via
# bupha. Rust floor + org docs contract (no xtask in this repo).
set -euo pipefail

# 16 cores / ~11 GiB RAM — uncapped cargo jobs OOM under earlyoom.
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"

cargo test --workspace

bash scripts/docs-check.sh
