#!/usr/bin/env bash
# Linux Viewer build (release). Oynali interfeys: target/release/viewer
# Kerak: sudo apt install build-essential pkg-config
# (oyna kutubxonalari - libwayland, libxkbcommon, libGL - ish vaqtida yuklanadi; Ubuntu desktop'da bor)
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release -p viewer
echo "Tayyor: target/release/viewer  (ishga tushirish: ./target/release/viewer)"
