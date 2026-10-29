# Remote Desktop

Cross-platform remote desktop. Reja va holat: [PLAN.md](PLAN.md).

## Tuzilma

| Crate | Vazifa |
|---|---|
| `crates/protocol` | Xabarlar, versiya, konstantalar (ikkala tomon uchun umumiy) |
| `crates/rdcore` | Transport (QUIC), autentifikatsiya, umumiy mantiq |
| `crates/host-linux` | Host: Ubuntu (Wayland), ekran olish va kiritishni bajarish |
| `crates/viewer-windows` | Viewer: Windows, ko'rsatish va kiritishni yuborish |

## Build

- Windows (Viewer): `scripts\build.ps1`
- Ubuntu (Host): `./scripts/build.sh`

Rust (stable) kerak: https://rustup.rs

## Bosqich 1 sinovi (ulanish va autentifikatsiya)

1. Avtomatik testlar (bir kompyuterda Host + Viewer): `cargo test --workspace`
2. Qo'lda sinash:
   - Ubuntu: `cargo run -p host-linux -- set-password`, keyin `cargo run -p host-linux -- run`
     (fingerprint ekranga chiqadi; Ubuntu firewall'da bo'lsa: `sudo ufw allow 7777/udp`)
   - Windows: `cargo run -p viewer-windows -- connect <Ubuntu IP>`
     (birinchi marta fingerprint'ni Host ekranidagi bilan solishtirib `yes` deng)
