# Remote Desktop: loyiha rejasi

> Bu fayl ish davomida yangilanadi. Vazifa bajarilsa `[ ]` ni `[x]` ga o'zgartiring va pastdagi "Jurnal" bo'limiga bir qator yozing.
> Holat belgilari: `[ ]` boshlanmagan, `[~]` kod yozilgan, lekin foydalanuvchi o'z kompyuterida build/test qilib tasdiqlashi kerak, `[x]` tayyor va tekshirilgan, `[!]` to'siq bor.
> Build yo'li: Claude tashqi kutubxonalarni yuklay olmaydi (crates.io yopiq), shuning uchun kompilyatsiya va testlar foydalanuvchi kompyuterida (`cargo test --workspace`) bajariladi.

## 1. Maqsad

Cross-platform remote desktop dastur. Microsoft RDP mantig'i (ekranni kanallar orqali uzatish, kiritishni teskari yuborish, ulanishda sozlamalarni kelishish), lekin **alohida remote sessiya yaratilmaydi**: Host'dagi joriy ekran uzatiladi.

Asosiy funksiyalar:
1. Ekran ma'lumotini uzatish (real vaqt, video)
2. Sichqoncha (va klaviatura) boshqaruvi
3. Ekran sozlamalari (ruxsat, FPS, sifat, monitor)

## 2. Qabul qilingan qarorlar

| Mavzu | Qaror |
|---|---|
| **Host (boshqariladigan)** | **Linux: Ubuntu, GNOME, Wayland** |
| **Viewer (boshqaruvchi)** | **Windows** |
| Keyinroq (ixtiyoriy) | Windows Host, Linux Viewer. macOS kerak emas |
| Host muhiti | Ubuntu 26.04.1 LTS, kernel 7.0.0-42-generic. Host qarovsiz ishlaydi (oldida hech kim bo'lmaydi). Oq IP statik |
| Ulanish | Internet orqali, to'g'ridan-to'g'ri `IP:port`. Host'da oq (public) IP bor |
| Server | Signaling va relay yo'q. Host tinglaydi, Viewer ulanadi |
| Til | Rust (yagona binary, tashqi runtime kerak emas) |
| Transport | QUIC (`quinn`), TLS 1.3 shifrlash. Zaxira: TCP + TLS |
| Video | H.264 (apparat kodlash bo'lsa ishlatiladi, aks holda dasturiy) |
| UI | Tauri (Rust + web UI) |
| Autentifikatsiya | Self-signed sertifikat + fingerprint tasdig'i + parol/PIN |
| Qo'shimcha o'rnatish | Talab qilinmaydi. Dastur o'zi yetarli bo'lishi shart |

## 3. Savollar

Javob berilgan:
- [x] **Q1.** Host'da oq IP bormi? Ha. Internet orqali to'g'ridan-to'g'ri ulanish ishlaydi, relay/VPN kerak emas.
- [x] **Q2.** Linux Host, Windows Viewer. Hozirgi rejada shu.
- [x] **Q3.** Linux: standart Ubuntu (GNOME, Wayland).

Qo'shimcha savollar (javob berilgan):
- [x] **Q4.** Build va test: foydalanuvchi o'zi qiladi, Ubuntu yonida. Kerak bo'lsa LAN orqali ulanib sinaladi.
- [x] **Q5.** Oq IP statik. DDNS kerak emas.
- [x] **Q6.** Host oldida hech kim bo'lmaydi. Demak qarovsiz (unattended) ishlash **majburiy talab**. Bu Wayland'da eng xavfli joy, shuning uchun Bosqich 2 boshida go/no-go tekshiruv bor.
- [x] **Q7.** Ubuntu 26.04.1 LTS, kernel 7.0.0-42-generic. Portal imkoniyatlari shu versiyada tekshiriladi.

## 4. Arxitektura

```
[Viewer: Windows] <--- QUIC (shifrlangan, internet) ---> [Host: Ubuntu Wayland]
  dekodlaydi, ko'rsatadi                                   portal orqali ekranni oladi, kodlaydi
  kiritishni yuboradi                                      portal orqali kiritishni bajaradi
```

Bitta kod bazasi, ikki rol. Hozir: Linux = Host, Windows = Viewer.

**Ulanish ichidagi kanallar (RDP virtual channel'lariga o'xshash):**
- `control`: handshake, capability negotiation, sozlamalarni o'zgartirish, ping
- `video`: kodlangan kadrlar (Host -> Viewer)
- `input`: sichqoncha va klaviatura hodisalari (Viewer -> Host)
- `clipboard`: ixtiyoriy, keyingi bosqich

**Koordinatalar:** nisbiy (0.0 .. 1.0), shunda ekran o'lchamlari farq qilsa ham to'g'ri ishlaydi.

**Platforma qatlami:**

| | Linux Host (hozir) | Windows Viewer (hozir) | Windows Host (keyinroq) |
|---|---|---|---|
| Ekran | xdg-desktop-portal ScreenCast + PipeWire | dekodlash va ko'rsatish | DXGI Desktop Duplication |
| Kiritish | RemoteDesktop portali (yoki libei) bajaradi | hodisalarni ushlab yuboradi | SendInput |
| Eslatma | Birinchi ruxsat dialogi shart, restore token bilan eslab qolinadi | Ko'p murakkabligi yo'q | UAC/login uchun ixtiyoriy xizmat |

## 5. Bosqichlar va vazifalar

### Bosqich 0. Tayyorgarlik
- [x] Build va test muhiti belgilandi (Q4): Windows va Ubuntu foydalanuvchi qo'lida, LAN orqali sinash mumkin
- [ ] Rust toolchain: Windows'da (Viewer) va Ubuntu'da (Host). **Foydalanuvchi o'rnatadi** (https://rustup.rs), `cargo --version` bilan tekshiradi. Windows'da qo'shimcha: Visual Studio Build Tools ("Desktop development with C++"), aks holda `link.exe not found` xatosi chiqadi. Ubuntu'da: `sudo apt install build-essential`
- [x] Git repozitoriy yaratish (`git init`, birinchi commit hali yo'q)
- [x] Cargo workspace: `protocol`, `rdcore`, `host-linux`, `viewer-windows` (UI `app` keyin qo'shiladi). `core` nomi Rust'ning o'z kutubxonasi bilan to'qnashgani uchun `rdcore` deb nomlandi
- [x] Build skriptlari: `scripts/build.ps1` (Windows), `scripts/build.sh` (Ubuntu)

### Bosqich 1. Poydevor: protokol va ulanish
- [x] Protokol xabarlari: hello, auth, capabilities, ping/pong, bye (`crates/protocol`, 11 ta test o'tdi). Input va settings xabarlari Bosqich 3 va 5 da qo'shiladi
- [x] Binary serializatsiya: qo'lda yozilgan, tashqi kutubxonasiz, frame (u32 uzunlik) + xabar turi; `PROTOCOL_VERSION = 1`
- [~] QUIC server (Host) va klient (Viewer): `rdcore::server`, `rdcore::client` (quinn 0.11, TLS 1.3). Hozircha faqat control oqimi
- [~] Self-signed sertifikat yaratish va saqlash (Host): `rdcore::cert` (rcgen)
- [~] Fingerprint ko'rsatish va "known hosts" saqlash (Viewer). `known_hosts` mantiqi tekshirilgan (3 test), ulanish bilan bog'lanishi `[~]`. Fingerprint o'zgarsa ulanish rad etiladi
- [~] Parol autentifikatsiyasi: parol yuborilmaydi, `HMAC(argon2id(parol), nonce || fingerprint)` isboti yuboriladi (fingerprint bog'langani MITM'dan himoya qiladi). `rdcore::auth`
- [~] Noto'g'ri urinishlarni cheklash: IP bo'yicha 10 daqiqada 5 xato -> 15 daqiqa blok. Mantiq tekshirilgan (5 test), Host'ga ulanishi `[~]`
- [~] Capability: Host kodek, maksimal FPS va monitorlar ro'yxatini e'lon qiladi (monitorlar hozircha bo'sh). Viewer tanlovi Bosqich 5 da (Settings xabari)
- [ ] Test (foydalanuvchi): 1) `cargo test --workspace` ikkala kompyuterda; 2) Ubuntu'da `host-linux run`, Windows'dan `viewer-windows connect <IP>`, ping ishlaydi; 3) noto'g'ri parol rad etiladi
  - Avtomatik loopback testlari yozilgan: `crates/rdcore/tests/loopback.rs` (muvaffaqiyatli ulanish va ping, noto'g'ri parol, blok, tasdiqlanmagan Host, o'zgargan fingerprint)

### Bosqich 2. Linux Host: ekran olish va kodlash

**Go/no-go tekshiruv (birinchi qilinadi).** Q6 sababli qarovsiz ishlash Ubuntu 26.04'da amalda ishlashi kerak. Kichik prototip bilan tekshiramiz, shundan keyingina qolgan ishga o'tamiz.
- [ ] Prototip: ScreenCast + RemoteDesktop portallarini bitta sessiyada ochish
- [ ] Birinchi marta ruxsat berish (foydalanuvchi oldida), restore token saqlash
- [ ] Qayta ishga tushirishdan keyin token bilan **dialogsiz** sessiya ochilishini tekshirish (ham ekran, ham kiritish)
- [ ] Ekran qulflanganda, o'chganda va uyquga ketganda holatni tekshirish
- [ ] Natijani hujjatlash. Agar dialogsiz ishlamasa: muqobil yo'l tanlanadi (masalan GNOME'ning o'z remote desktop xizmati asosida, yoki libei) va reja yangilanadi

- [ ] ScreenCast portali orqali sessiya ochish (GNOME'da ruxsat dialogi)
- [ ] PipeWire oqimidan kadr olish
- [ ] Portal restore token'ni saqlash (keyingi ishga tushishlarda dialogsiz)
- [ ] Monitor ro'yxati va tanlash
- [ ] H.264 kodlash (dasturiy zaxira)
- [ ] Apparat kodlash aniqlash (VAAPI/NVENC, mavjud bo'lsa)
- [ ] Video oqimini Viewer'ga yuborish
- [ ] Test: Ubuntu'dan lokal tarmoqda barqaror video, kechikish o'lchovi

### Bosqich 3. Windows Viewer: ko'rsatish
- [ ] Video oqimini qabul qilish va dekodlash (apparat dekodlash: D3D11/DXVA)
- [ ] Viewer oynasida ko'rsatish (to'liq ekran, oynaga moslash, masshtab)
- [ ] Ulanish holati va xatolarni ko'rsatish
- [ ] Test: Windows oynasida Ubuntu ekrani ko'rinadi

### Bosqich 4. Boshqaruv (sichqoncha va klaviatura)
- [ ] Viewer'da sichqoncha harakati, bosish, g'ildirakni ushlash
- [ ] Viewer'da klaviatura hodisalari (scancode va layout masalasi, maxsus tugmalar)
- [ ] Nisbiy koordinata yuborish
- [ ] Host'da RemoteDesktop portali orqali sichqoncha va klaviaturani bajarish
- [ ] Ko'p monitorli Host'da koordinatani to'g'ri moslashtirish
- [ ] Kursor ko'rinishini uzatish
- [ ] Test: Windows'dan Ubuntu'ni boshqarish, input kechikishi o'lchovi

### Bosqich 5. Sozlamalar va moslashuv
- [ ] Ruxsat (resolution) va masshtab tanlash
- [ ] FPS tanlash (15/30/60)
- [ ] Sifat rejimi: tezlik / sifat / avtomatik
- [ ] Monitor tanlash (Host'da bir nechta bo'lsa)
- [ ] Ulanish davomida sozlamani o'zgartirish (qayta ulanmasdan)
- [ ] Avtomatik bitrate boshqaruvi (internet sifatiga qarab)
- [ ] TCP + TLS zaxira rejimi (UDP bloklangan tarmoqlar uchun)
- [ ] Test: internet orqali ulanishda sifat va barqarorlik

### Bosqich 6. Internet ulanishi va xavfsizlik
- [ ] Host routerida UDP port forwarding (Linux Host uchun yo'riqnoma)
- [ ] Ubuntu firewall (ufw) qoidasi
- [x] IP statik (Q5), DDNS kerak emas
- [ ] Ubuntu qarovsiz ishlash sozlamalari: avtomatik kirish (auto-login), ekran qulfini o'chirish, ekran o'chishi va uyquni o'chirish
- [ ] Dastur avtostarti (systemd user service), xato bo'lsa qayta ishga tushirish
- [ ] Parol kuchliligi talabi, parol xesh ko'rinishida saqlanadi
- [ ] Ixtiyoriy IP ruxsat ro'yxati (allowlist)
- [ ] Test: haqiqiy internet orqali boshqa tarmoqdan ulanish

### Bosqich 7. UI va tayyorlash
- [ ] Host UI (Linux): IP, port, parol, ulanish holati, uzish tugmasi
- [ ] Host'da ulanish tasdig'i va doimiy indikator
- [ ] Viewer UI (Windows): manzil kiritish, saqlangan Host'lar, sozlamalar paneli
- [ ] Windows: portable `.exe` va installer
- [ ] Linux: AppImage yoki `.deb`, avtostart (systemd user service)
- [ ] Ixtiyoriy: clipboard sinxronizatsiyasi

### Bosqich 8. Keyinroq (ixtiyoriy, hozirgi reja doirasida emas)
- [ ] Windows Host (DXGI + SendInput)
- [ ] Linux Viewer
- [ ] Windows xizmati (UAC va login ekrani uchun)
- [ ] Relay (agar oq IP bo'lmagan holat chiqsa)

## 6. Xavfsizlik talablari

- [ ] Barcha trafik shifrlangan
- [ ] Parolsiz ulanish yo'q
- [ ] Host har ulanishni ko'rsatadi va uzish imkoniyati bor
- [ ] Brute-force'ga qarshi cheklov
- [ ] Parol xesh ko'rinishida saqlanadi (ochiq matnda emas)

## 7. Xavflar

| Xavf | Yengillashtirish |
|---|---|
| Qarovsiz ishlash (Q6): portal ruxsati birinchi marta qo'lda beriladi | Bir marta Host oldida ruxsat berish, restore token, Bosqich 2 boshidagi go/no-go tekshiruv |
| Restore token qayta ishga tushirishdan keyin ishlamasligi mumkin | Erta sinash; muqobil yo'l (GNOME remote desktop xizmati, libei) |
| Ekran qulflansa yoki o'chsa video qora bo'ladi, GDM login ekrani ko'rinmaydi | Auto-login, ekran qulfi va uyqu o'chiriladi; foydalanuvchi sessiyasi doim ochiq |
| Ubuntu 26.04 yangi, portal xatti-harakati o'zgargan bo'lishi mumkin | Real muhitda sinash (Q4: Ubuntu yonida) |
| Kechikish | Apparat kodlash, o'zgargan qismlarni yuborish, o'lchash |
| Internetga ochiq port | Kuchli parol, rate limit, allowlist, fingerprint |

## 8. Jurnal

| Sana | Nima qilindi |
|---|---|
| 2026-10-29 | Reja tuzildi, qarorlar qabul qilindi |
| 2026-10-29 | Bosqich 0 deyarli tayyor: workspace, git, build skriptlari yaratildi. Bo'sh workspace Linux'da `cargo test` va `cargo build --release` bilan tekshirildi. Qolgan: foydalanuvchi Rust'ni o'rnatadi |
| 2026-10-29 | Bosqich 1 kodi yozildi (protokol, auth, QUIC server/klient, CLI). Protokol va std-only modullarning 23 ta testi Claude tomonda o'tdi, tarmoq/kripto qismi faqat sintaksis jihatdan tekshirildi. Kompilyatsiya va testlar foydalanuvchi kompyuterida kutilmoqda |
| 2026-10-29 | Qaror: foydalanuvchida akkaunt sozlamasiga kirish yo'q, shuning uchun lokal build yo'li tanlandi |
| 2026-10-29 | **To'siq (hal qilindi lokal build bilan):** Claude ishlaydigan muhitdan crates.io ga ulanib bo'lmaydi (tarmoq ruxsati yo'q), shuning uchun tashqi kutubxonali kod (QUIC va boshqalar) Claude tomonda kompilyatsiya qilinmaydi. Yechim kutilmoqda |
| 2026-10-29 | Q4-Q7 javoblari qo'shildi: Ubuntu 26.04.1, statik IP, qarovsiz ishlash talabi. Bosqich 2 boshiga go/no-go tekshiruv qo'shildi |
| 2026-10-29 | Q1-Q3 javoblari qo'shildi: Linux (Ubuntu GNOME Wayland) Host, Windows Viewer, oq IP bor. Bosqichlar shunga qayta tartiblandi |
