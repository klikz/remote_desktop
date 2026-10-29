#!/usr/bin/env bash
# Qarovsiz (unattended) Host uchun GNOME sozlamalari:
# ekran qulfi, ekran o'chishi (blank) va uyquni o'chiradi, chunki GNOME'da ekran qulflanganda
# yoki o'chganda ScreenCast/RemoteDesktop sessiyalari yopiladi yoki yangisini ochib bo'lmaydi.
#
#   bash scripts/ubuntu-host-setup.sh            sozlamalarni qo'llash (eski qiymatlar saqlanadi)
#   bash scripts/ubuntu-host-setup.sh --revert   eski qiymatlarni qaytarish
#   bash scripts/ubuntu-host-setup.sh --status   joriy qiymatlarni ko'rsatish
set -euo pipefail

BACKUP="${XDG_CONFIG_HOME:-$HOME/.config}/remote-desktop/gnome-settings.backup"

# schema|kalit|qiymat
SETTINGS=(
  "org.gnome.desktop.screensaver|lock-enabled|false"
  "org.gnome.desktop.session|idle-delay|0"
  "org.gnome.desktop.lockdown|disable-lock-screen|true"
  "org.gnome.settings-daemon.plugins.power|sleep-inactive-ac-type|'nothing'"
  "org.gnome.settings-daemon.plugins.power|sleep-inactive-battery-type|'nothing'"
)

have_key() { gsettings get "$1" "$2" >/dev/null 2>&1; }

case "${1:-apply}" in
  --status)
    for item in "${SETTINGS[@]}"; do
      IFS='|' read -r schema key want <<<"$item"
      if have_key "$schema" "$key"; then
        printf '%-45s %-32s hozir=%s kerak=%s\n' "$schema" "$key" "$(gsettings get "$schema" "$key")" "$want"
      else
        printf '%-45s %-32s (bu tizimda yo'"'"'q)\n' "$schema" "$key"
      fi
    done
    ;;
  --revert)
    if [[ ! -f "$BACKUP" ]]; then
      echo "Zaxira fayli topilmadi: $BACKUP (qaytariladigan narsa yo'q)"; exit 1
    fi
    while IFS='|' read -r schema key old; do
      [[ -z "$schema" ]] && continue
      gsettings set "$schema" "$key" "$old" && echo "qaytarildi: $schema $key = $old"
    done <"$BACKUP"
    rm -f "$BACKUP"
    ;;
  apply|--apply)
    mkdir -p "$(dirname "$BACKUP")"
    if [[ -f "$BACKUP" ]]; then
      echo "Eslatma: zaxira allaqachon bor ($BACKUP), asl qiymatlar qayta yozilmaydi."
    else
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
      else
        echo "o'tkazib yuborildi (kalit yo'q): $schema $key"
      fi
    done
    echo
    echo "Qo'lda qiling: Settings > System > Users > Automatic Login (qayta yuklashdan keyin avtomatik kirish uchun)."
    ;;
  *)
    echo "Foydalanish: $0 [--apply|--revert|--status]"; exit 2 ;;
esac
