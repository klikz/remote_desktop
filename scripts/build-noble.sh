#!/usr/bin/env bash
# Ubuntu 24.04 va undan yangi tizimlar uchun release build (Docker orqali).
# Birinchi ishga tushirishda build muhiti (image) yaratiladi; keyingilari tez: Rust va kutubxonalar
# qayta yuklanmaydi, faqat o'zgargan kod kompilyatsiya qilinadi.
#
# Foydalanish:  bash scripts/build-noble.sh [paket]      (standart: viewer)
#               bash scripts/build-noble.sh host-linux  (Linux Host: target-noble/release/host-linux)
#               REBUILD_IMAGE=1 bash scripts/build-noble.sh   (build muhitini qayta yaratish)
# Natija:       target-noble/release/<paket>
set -euo pipefail
cd "$(dirname "$0")/.."

PKG="${1:-viewer}"
IMG=rd-build-noble
DOCKER=docker
if ! docker info >/dev/null 2>&1; then
    DOCKER="sudo docker"
fi

if [ "${REBUILD_IMAGE:-0}" = "1" ] || ! $DOCKER image inspect "$IMG" >/dev/null 2>&1; then
    echo "==> Build muhiti yaratilmoqda (faqat bir marta, bir necha daqiqa)..."
    $DOCKER build -t "$IMG" -f docker/Dockerfile.noble docker
fi

echo "==> $PKG build qilinmoqda..."
$DOCKER run --rm \
    -e OWNER="$(id -u):$(id -g)" \
    -v "$PWD":/src \
    -v rd-cargo-registry:/opt/cargo/registry \
    -v rd-cargo-git:/opt/cargo/git \
    -w /src "$IMG" \
    bash -c '
set -e
# viewer: avval Linux Host (host-linux) build qilinadi va viewer ichiga joylanadi,
# shunda bitta fayl ham Viewer, ham Host o'"'"'rnatuvchisi bo'"'"'ladi.
if [ "$0" = "viewer" ]; then
  cargo build --release -p host-linux --target-dir target-noble
  export RD_HOST_BIN=/src/target-noble/release/host-linux
fi
cargo build --release -p "$0" --target-dir target-noble
chown -R "$OWNER" target-noble' "$PKG"

echo "==> Tayyor: $PWD/target-noble/release/$PKG"
if [ "$PKG" = "viewer" ]; then
    echo "    (ichida Linux Host ham bor: \"Bu kompyuter\" > \"Ulanishga ruxsat berish\" uni o'zi o'rnatadi)"
fi
