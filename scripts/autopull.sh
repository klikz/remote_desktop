#!/usr/bin/env bash
# Ubuntu (ishlab chiqish uchun): GitHub'dagi yangi commit'larni olib, Host'ni yangilaydi.
#  - service o'rnatilgan bo'lsa (install-host.sh): release build, o'rnatadi va service'ni qayta ishga tushiradi
#  - aks holda: debug Host'ni shu terminalda ishga tushiradi va yangilanganda qayta turg'izadi
# Foydalanish: bash scripts/autopull.sh     (RD_DEBUG=1 faqat service'siz rejimda ta'sir qiladi)
set -u
cd "$(dirname "$0")/.."
BIN="$HOME/.local/bin/remote-desktop-host"

if systemctl --user is-enabled remote-desktop-host >/dev/null 2>&1; then
    MODE=service
    echo "[autopull] service rejimi: yangilanishda remote-desktop-host qayta ishga tushiriladi"
    echo "[autopull] loglar: journalctl --user -u remote-desktop-host -f"
else
    MODE=direct
    cargo build -p host-linux || exit 1
fi

HOST_PID=""
start_host() { ./target/debug/host-linux run & HOST_PID=$!; }
trap '[ -n "$HOST_PID" ] && kill $HOST_PID 2>/dev/null; exit 0' INT TERM
[ "$MODE" = direct ] && start_host

update() {
    if [ "$MODE" = service ]; then
        cargo build --release -p host-linux || return 1
        install -m 755 target/release/host-linux "$BIN.new" && mv -f "$BIN.new" "$BIN"
        systemctl --user restart remote-desktop-host && echo "[autopull] service qayta ishga tushdi"
    else
        cargo build -p host-linux || return 1
        kill $HOST_PID 2>/dev/null; wait $HOST_PID 2>/dev/null
        start_host
    fi
}

while sleep 10; do
    git fetch -q origin main || continue
    if [ "$(git rev-parse HEAD)" != "$(git rev-parse origin/main)" ]; then
        echo "[autopull] yangi o'zgarish: $(git log --oneline -1 origin/main)"
        git reset -q --hard origin/main
        update || echo "[autopull] build xatosi, eski Host ishlashda davom etadi"
    fi
done
