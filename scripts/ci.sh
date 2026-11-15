#!/usr/bin/env bash
# ci-gate gate for main — detached worktree of the pushed sha via
# bupha. Rust floor + org docs contract (no xtask in this repo).
set -euo pipefail
export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"

# 16 cores / ~11 GiB RAM — cargo jobs OOM under earlyoom on this host.
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
export RUSTC_WRAPPER="${RUSTC_WRAPPER:-sccache}"
export SCCACHE_CONF="${SCCACHE_CONF:-file://$HOME/.cache/sccache}"

cargo test --workspace

bash scripts/docs-check.sh
