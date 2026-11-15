#!/usr/bin/env bash
# ci-gate gate for main — detached worktree of the pushed sha via
# bupha. Rust floor + org docs contract (no xtask in this repo).
set -euo pipefail

# 16 cores / ~11 GiB RAM — cargo jobs OOM under earlyoom on this host.
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"

cargo test --workspace

bash scripts/docs-check.sh
