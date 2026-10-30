#!/usr/bin/env bash
# Ubuntu Host'ni TAYYOR (compile qilingan) fayldan o'rnatadi: loyiha kodi, Rust yoki cargo kerak emas.
#
# Kerakli fayllar bir papkada bo'lsin:
#   install-host-binary.sh
#   host-linux              (build-noble.sh host-linux natijasi: target-noble/release/host-linux)
#
#   bash install-host-binary.sh                  o'rnatish / yangilash
#   bash install-host-binary.sh --autologin      + GDM avtomatik kirish (sudo so'raladi)
#   bash install-host-binary.sh --status         holatni ko'rsatish
#   bash install-host-binary.sh --uninstall      xizmatni o'chirish (sozlamalar qoladi)
#   bash install-host-binary.sh [opts] /yo'l/host-linux    fayl boshqa joyda bo'lsa
#
# Nima qiladi: kerakli paketlarni (GStreamer, PipeWire, portal) o'rnatadi, faylni
# ~/.local/bin/remote-desktop-host ga qo'yadi, parol so'raydi, GNOME qulf/uyqu/ekran o'chishini
# o'chiradi va systemd user xizmatini yoqadi (login qilinganda o'zi ishga tushadi).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="$HOME/.local/bin/remote-desktop-host"
UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
UNIT="$UNIT_DIR/remote-desktop-host.service"
CONF="${XDG_CONFIG_HOME:-$HOME/.config}/remote-desktop"
BACKUP="$CONF/gnome-settings.backup"
GDM_CONF="/etc/gdm3/custom.conf"

MODE=install
AUTOLOGIN=0
SRC=""
for a in "$@"; do
  case "$a" in
    --autologin) AUTOLOGIN=1 ;;
    --status) MODE=status ;;
    --uninstall) MODE=uninstall ;;
    -*) echo "Noma'lum parametr: $a"; exit 2 ;;
    *) SRC="$a" ;;
  esac
done

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

# GNOME: ekran qulfi, ekran o'chishi va uyquni o'chirish (eski qiymatlar zaxiraga yoziladi).
SETTINGS=(
  "org.gnome.desktop.screensaver|lock-enabled|false"
  "org.gnome.desktop.session|idle-delay|0"
  "org.gnome.desktop.lockdown|disable-lock-screen|true"
  "org.gnome.settings-daemon.plugins.power|sleep-inactive-ac-type|'nothing'"
  "org.gnome.settings-daemon.plugins.power|sleep-inactive-battery-type|'nothing'"
)
have_key() { gsettings get "$1" "$2" >/dev/null 2>&1; }
apply_gnome() {
  mkdir -p "$CONF"
  if [[ ! -f "$BACKUP" ]]; then
    for item in "${SETTINGS[@]}"; do
      IFS='|' read -r schema key want <<<"$item"
      have_key "$schema" "$key" && echo "$schema|$key|$(gsettings get "$schema" "$key")" >>"$BACKUP"
    done
    echo "Asl qiymatlar saqlandi: $BACKUP"
  fi
  for item in "${SETTINGS[@]}"; do
    IFS='|' read -r schema key want <<<"$item"
    if have_key "$schema" "$key"; then
      gsettings set "$schema" "$key" "$want" && echo "o'rnatildi: $schema $key = $want"
    fi
  done
}

case "$MODE" in
  status)
    echo "Dastur:          $( [[ -x "$BIN" ]] && echo "$BIN" || echo "o'rnatilmagan")"
    echo "Parol:           $( [[ -f "$CONF/password.key" ]] && echo "o'rnatilgan" || echo "YO'Q")"
    echo "Service:         $(systemctl --user is-enabled remote-desktop-host 2>/dev/null || echo yo\'q) / $(systemctl --user is-active remote-desktop-host 2>/dev/null || true)"
    echo "Avtomatik kirish: $(autologin_status)"
    PORT="$(tr -d '[:space:]' <"$CONF/port" 2>/dev/null || true)"; PORT="${PORT:-7777}"
    echo "UDP $PORT:        $(ss -ulpn 2>/dev/null | grep -q ":$PORT " && echo tinglanmoqda || echo tinglanmayapti)"
    echo "TCP $PORT:        $(ss -tlpn 2>/dev/null | grep -q ":$PORT " && echo tinglanmoqda || echo tinglanmayapti)"
    echo "Loglar: journalctl --user -u remote-desktop-host -f"
    exit 0 ;;
  uninstall)
    systemctl --user disable --now remote-desktop-host 2>/dev/null || true
    rm -f "$UNIT"; systemctl --user daemon-reload
    echo "Xizmat o'chirildi. Dastur ($BIN), parol va sozlamalar joyida qoldi."
    exit 0 ;;
esac

# --- Tayyor faylni topish
if [[ -z "$SRC" ]]; then
  for c in "$HERE/host-linux" "$HERE/remote-desktop-host" "./host-linux"; do
    [[ -f "$c" ]] && SRC="$c" && break
  done
fi
if [[ -z "$SRC" || ! -f "$SRC" ]]; then
  echo "Host fayli (host-linux) topilmadi. Uni shu skript yoniga qo'ying yoki yo'lini bering:"
  echo "  bash $0 /yo'l/host-linux"
  exit 1
fi

echo "== 1/5 Kerakli paketlar =="
PKGS=(gstreamer1.0-tools gstreamer1.0-pipewire gstreamer1.0-plugins-base gstreamer1.0-plugins-good
      gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly pipewire xdg-desktop-portal xdg-desktop-portal-gnome)
MISSING=()
for p in "${PKGS[@]}"; do
  dpkg -s "$p" >/dev/null 2>&1 || MISSING+=("$p")
done
if ((${#MISSING[@]})); then
  echo "O'rnatiladi: ${MISSING[*]} (sudo parol so'ralishi mumkin)"
  sudo apt-get update
  sudo apt-get install -y "${MISSING[@]}"
else
  echo "Hammasi o'rnatilgan."
fi

echo "== 2/5 O'rnatish: $BIN =="
mkdir -p "$(dirname "$BIN")"
install -m 755 "$SRC" "$BIN.new"
mv -f "$BIN.new" "$BIN"

echo "== 3/5 Parol =="
if [[ -f "$CONF/password.key" ]]; then
  echo "Parol allaqachon o'rnatilgan (o'zgartirish: $BIN set-password)"
else
  "$BIN" set-password
fi

echo "== 4/5 GNOME sozlamalari (qulf, ekran o'chishi, uyqu) =="
apply_gnome
[[ "$AUTOLOGIN" == "1" ]] && enable_autologin

echo "== 5/5 systemd user xizmati =="
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
echo "Avtomatik kirish: $(autologin_status)$( [[ "$(autologin_status)" == "o'chiq" ]] && echo "  ->  yoqish: bash $0 --autologin")"
echo "Ekranni birinchi marta ulashda GNOME ruxsat so'rashi mumkin; undan keyin eslab qoladi."
echo "Holat: bash $0 --status"
