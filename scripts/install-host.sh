#!/usr/bin/env bash
# Ubuntu Host'ni qarovsiz ishga o'rnatadi:
#   - release build va ~/.local/bin/remote-desktop-host ga o'rnatish
#   - parol (o'rnatilmagan bo'lsa so'raydi)
#   - GNOME: ekran qulfi / o'chishi / uyqu o'chiriladi (ubuntu-host-setup.sh)
#   - systemd user service: login qilinganda avtomatik ishga tushadi, yiqilsa qayta turadi
#   - --autologin: GDM avtomatik kirishni yoqadi (sudo so'raladi)
#
#   bash scripts/install-host.sh                 o'rnatish / yangilash
#   bash scripts/install-host.sh --autologin     + avtomatik kirish
#   bash scripts/install-host.sh --status        holatni ko'rsatish
#   bash scripts/install-host.sh --uninstall     service'ni o'chirish (sozlamalar qoladi)
set -euo pipefail
cd "$(dirname "$0")/.."

BIN="$HOME/.local/bin/remote-desktop-host"
UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
UNIT="$UNIT_DIR/remote-desktop-host.service"
CONF="${XDG_CONFIG_HOME:-$HOME/.config}/remote-desktop"
GDM_CONF="/etc/gdm3/custom.conf"

autologin_status() {
  if [[ -f "$GDM_CONF" ]] && grep -Eq '^\s*AutomaticLoginEnable\s*=\s*(true|True|1)' "$GDM_CONF" \
     && grep -Eq "^\s*AutomaticLogin\s*=\s*$USER\s*$" "$GDM_CONF"; then
    echo "yoqilgan ($USER)"
  else
    echo "o'chiq"
  fi
}

enable_autologin() {
  if [[ ! -f "$GDM_CONF" ]]; then
    echo "$GDM_CONF topilmadi. Settings > System > Users > Automatic Login orqali yoqing."; return
  fi
  echo "GDM avtomatik kirish yoqilmoqda ($USER). sudo parol so'ralishi mumkin."
  sudo cp -n "$GDM_CONF" "$GDM_CONF.rd-backup" || true
  sudo python3 - "$GDM_CONF" "$USER" <<'PY'
import sys, re
path, user = sys.argv[1], sys.argv[2]
lines = open(path).read().split('\n')
out, in_daemon, done = [], False, set()
for l in lines:
    s = l.strip()
    if s.startswith('['):
        if in_daemon:
            for k, v in (('AutomaticLoginEnable', 'true'), ('AutomaticLogin', user)):
                if k not in done: out.append(f'{k}={v}'); done.add(k)
        in_daemon = (s == '[daemon]')
        out.append(l); continue
    if in_daemon:
        m = re.match(r'#?\s*(AutomaticLoginEnable|AutomaticLogin)\s*=', s)
        if m:
            k = m.group(1)
            if k not in done:
                out.append(f'{k}=' + ('true' if k == 'AutomaticLoginEnable' else user)); done.add(k)
            continue
    out.append(l)
if in_daemon:
    for k, v in (('AutomaticLoginEnable', 'true'), ('AutomaticLogin', user)):
        if k not in done: out.append(f'{k}={v}'); done.add(k)
if not any(l.strip() == '[daemon]' for l in lines):
    out += ['[daemon]', 'AutomaticLoginEnable=true', f'AutomaticLogin={user}']
open(path, 'w').write('\n'.join(out))
PY
  echo "Avtomatik kirish: $(autologin_status). Zaxira: $GDM_CONF.rd-backup"
}

case "${1:-install}" in
  --status)
    echo "Dastur:          $( [[ -x "$BIN" ]] && echo "$BIN" || echo "o'rnatilmagan")"
    echo "Parol:           $( [[ -f "$CONF/password.key" ]] && echo "o'rnatilgan" || echo "YO'Q")"
    echo "Service:         $(systemctl --user is-enabled remote-desktop-host 2>/dev/null || echo yo\'q) / $(systemctl --user is-active remote-desktop-host 2>/dev/null || true)"
    echo "Avtomatik kirish: $(autologin_status)"
    PORT="$(tr -d '[:space:]' <"$CONF/port" 2>/dev/null || true)"; PORT="${PORT:-7777}"
    echo "Port:            $(ss -ulpn 2>/dev/null | grep -q ":$PORT " && echo "UDP $PORT tinglanmoqda" || echo "UDP $PORT tinglanmayapti")"
    echo
    bash scripts/ubuntu-host-setup.sh --status
    echo
    echo "Loglar: journalctl --user -u remote-desktop-host -f"
    exit 0 ;;
  --uninstall)
    systemctl --user disable --now remote-desktop-host 2>/dev/null || true
    rm -f "$UNIT"; systemctl --user daemon-reload
    echo "Service o'chirildi. Dastur ($BIN), parol va sozlamalar joyida qoldi."
    echo "GNOME sozlamalarini qaytarish: bash scripts/ubuntu-host-setup.sh --revert"
    exit 0 ;;
  install|--install|--autologin) ;;
  *) echo "Foydalanish: $0 [--autologin|--status|--uninstall]"; exit 2 ;;
esac

echo "== 1/5 Build (release) =="
cargo build --release -p host-linux

echo "== 2/5 O'rnatish: $BIN =="
mkdir -p "$(dirname "$BIN")"
install -m 755 target/release/host-linux "$BIN.new"
mv -f "$BIN.new" "$BIN"

echo "== 3/5 Parol =="
if [[ -f "$CONF/password.key" ]]; then
  echo "Parol allaqachon o'rnatilgan (o'zgartirish: $BIN set-password)"
else
  "$BIN" set-password
fi

echo "== 4/5 GNOME sozlamalari (qulf, ekran o'chishi, uyqu) =="
bash scripts/ubuntu-host-setup.sh --apply
if [[ "${1:-}" == "--autologin" ]]; then
  enable_autologin
fi

echo "== 5/5 systemd user service =="
mkdir -p "$UNIT_DIR"
cat >"$UNIT" <<UNITEOF
[Unit]
Description=Remote Desktop Host
After=graphical-session.target
PartOf=graphical-session.target

[Service]
ExecStart=$BIN run
Restart=always
RestartSec=5

[Install]
WantedBy=graphical-session.target
UNITEOF
systemctl --user daemon-reload
systemctl --user enable remote-desktop-host >/dev/null
systemctl --user restart remote-desktop-host
sleep 2
systemctl --user --no-pager --lines=8 status remote-desktop-host || true

echo
echo "Tayyor. Host har login qilinganda o'zi ishga tushadi."
echo "Avtomatik kirish: $(autologin_status)$( [[ "$(autologin_status)" == "o'chiq" ]] && echo '  ->  yoqish: bash scripts/install-host.sh --autologin')"
echo "Loglar: journalctl --user -u remote-desktop-host -f"
