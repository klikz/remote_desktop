#!/usr/bin/env bash
# Ubuntu: GitHub'dagi o'zgarishlarni olib, Host'ni qayta build qilib ishga tushiradi.
# Foydalanish: ./scripts/autopull.sh            (oddiy)
#              RD_DEBUG=1 ./scripts/autopull.sh (diagnostika bilan)
# Har 10 soniyada tekshiradi; yangi commit kelsa Host qayta ishga tushadi. To'xtatish: Ctrl+C
set -u
cd "$(dirname "$0")/.."
cargo build -p host-linux || exit 1
start_host() { ./target/debug/host-linux run & HOST_PID=$!; }
trap 'kill $HOST_PID 2>/dev/null; exit 0' INT TERM
start_host
while sleep 10; do
    git fetch -q origin main || continue
    if [ "$(git rev-parse HEAD)" != "$(git rev-parse origin/main)" ]; then
        echo "[autopull] yangi o'zgarish: $(git log --oneline -1 origin/main)"
        git reset -q --hard origin/main
        if cargo build -p host-linux; then
            kill $HOST_PID 2>/dev/null; wait $HOST_PID 2>/dev/null
            start_host
        else
            echo "[autopull] build xatosi, eski Host ishlashda davom etadi"
        fi
    fi
done
