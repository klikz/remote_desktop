#!/usr/bin/env bash
# Ubuntu Host build (release)
set -euo pipefail
cd "$(dirname "$0")/.."
cargo test --workspace
cargo build --release -p host-linux
echo "Tayyor: target/release/host-linux"
