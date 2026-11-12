#!/usr/bin/env bash
# Stage the tamias pack payload (T-012): release-build
# `tamias-driver-sturn` and drop it under
# `libraries/sturn/drivers/bin/<host-triple>/` — the bundled-driver
# dir `tamias install` registers into `<tamias-home>/drivers/`.
# Gitignored on purpose: the binary is a build artifact; the
# committed pack is just `tamias.yaml` + this script.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release -p sturn-driver

triple="$(rustc -vV | sed -n 's/^host: //p')"
triple="${triple:-x86_64-unknown-linux-gnu}"
dest_dir="libraries/sturn/drivers/bin/$triple"
mkdir -p "$dest_dir"

# CARGO_TARGET_DIR-aware: `cargo build` honors it, so the artifact
# lookup must too (default: ./target).
target_dir="${CARGO_TARGET_DIR:-$PWD/target}"
cp "$target_dir/release/tamias-driver-sturn" "$dest_dir/tamias-driver-sturn"
chmod 755 "$dest_dir/tamias-driver-sturn"

echo "pack staged: $dest_dir/tamias-driver-sturn"
echo
echo "install the sturn command (generated bins[] shim):"
echo "  tamias install --allow-drivers ./libraries/sturn"
echo "  # or a pinned tarball: tamias bundle pack libraries/sturn"
