# Remote Desktop

Cross-platform remote desktop. Reja va holat: [PLAN.md](PLAN.md).

## Tuzilma

| Crate | Vazifa |
|---|---|
| `crates/protocol` | Xabarlar, versiya, konstantalar (ikkala tomon uchun umumiy) |
| `crates/rdcore` | Transport (QUIC), autentifikatsiya, umumiy mantiq |
| `crates/host-linux` | Host: Ubuntu (Wayland), ekran olish va kiritishni bajarish |
| `crates/viewer` | Viewer (Windows va Linux): oynali interfeys, ko'rsatish va kiritishni yuborish |

## Build

- Windows (Viewer): `scripts\build.ps1`
- Ubuntu (Host): `./scripts/build.sh`

Rust (stable) kerak: https://rustup.rs

## Bosqich 1 sinovi (ulanish va autentifikatsiya)

1. Avtomatik testlar (bir kompyuterda Host + Viewer): `cargo test --workspace`
2. Qo'lda sinash:
   - Ubuntu: `cargo run -p host-linux -- set-password`, keyin `cargo run -p host-linux -- run`
     (fingerprint ekranga chiqadi; Ubuntu firewall'da bo'lsa: `sudo ufw allow 7777/udp`)
   - Windows: `cargo run -p viewer` (oynali) yoki `cargo run -p viewer -- connect <Ubuntu IP>`
     (birinchi marta fingerprint'ni Host ekranidagi bilan solishtirib `yes` deng)

## Bosqich 2 sinovi (Ubuntu Host: portal va ekran olish)

Ruxsat va kiritish (qo'shimcha paket kerak emas):
`cargo run -p portal-probe` (birinchi marta GNOME dialogi chiqadi, keyingilarida chiqmaydi)

Qarovsiz Host uchun GNOME sozlamalari: `bash scripts/ubuntu-host-setup.sh` (`--status`, `--revert`).

Kadr olish va H.264 kodlash (GStreamer kerak):
```
sudo apt install libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev pkg-config \
  gstreamer1.0-pipewire gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
  gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly
cargo run -p portal-probe --features capture -- --capture 10 --out /tmp/test.h264
```
Majburiy kodlagich: `RD_ENCODER=x264enc cargo run ...`

## Video yuborish sinovi (Ubuntu Host -> Windows Viewer)

GStreamer dev paketlari o'rnatilgan bo'lishi kerak (yuqorida). Endi `host-linux` ham GStreamer'ga bog'liq, shuning uchun Ubuntu'da `cargo test --workspace` uchun ham ular kerak.

1. Ubuntu: `cargo run -p host-linux -- run` (birinchi marta GNOME ruxsat dialogi chiqadi)
2. Windows: `cargo run -p viewer -- connect <Ubuntu IP> --video 10 --out test.h264`
3. Statistika chiqadi. `test.h264` ni VLC bilan oching.
