#!/usr/bin/env bash
# Linux Viewer build (release). Oynali interfeys: target/release/viewer
# Kerak: sudo apt install build-essential pkg-config + GStreamer dev paketlari (Host uchun, README ga qarang).
# Viewer ichiga Linux Host ham joylanadi: "Ulanishga ruxsat berish" tugmasi uni o'zi o'rnatadi.
# (Faqat Viewer kerak bo'lsa va GStreamer dev paketlari yo'q bo'lsa: NO_HOST=1 bash scripts/build-viewer.sh)
set -euo pipefail
cd "$(dirname "$0")/.."
if [ "${NO_HOST:-0}" != "1" ]; then
  cargo build --release -p host-linux
  export RD_HOST_BIN="$PWD/target/release/host-linux"
fi
cargo build --release -p viewer
echo "Tayyor: target/release/viewer  (ishga tushirish: ./target/release/viewer)"
