# MS Remote — Android client

Faqat **client** (ko'rish va boshqarish): telefon relay server orqali 9 xonali ID bilan kompyuterga ulanadi.
Interfeys — Kotlin (oddiy Android View'lar), tarmoq/protokol — Rust (`rdcore`, JNI orqali), video — `MediaCodec`.

```
mobile_viewer/
  core/   Rust cdylib (librdmobile.so), alohida Cargo workspace; ../../crates/{rdcore,protocol} ga path bog'lanish
  app/    Android (Gradle, Kotlin) ilovasi
```

Asosiy workspace (Windows/Linux build, Makefile) bu papkaga bog'liq emas. Asosiy koddagi yagona o'zgarish:
`crates/rdcore/src/paths.rs` ga Android shoxi (config papkasi `RD_CONFIG_DIR` orqali, uni ilova `Native.init(filesDir)` beradi).

## Talablar

- Android Studio (yoki JDK 17 + Android SDK): `platforms;android-35`, `build-tools`, **NDK** (27.x sinalgan)
- Rust: `rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android`
- `cargo install cargo-ndk`
- `ANDROID_NDK_HOME` — NDK papkasi (masalan `~/AppData/Local/Android/Sdk/ndk/27.1.12297006`)
- Gradle 8.9 (Android Studio o'zi yuklaydi; konsolda ishlatish uchun bir marta `gradle wrapper`)

## Build

Loyiha ildizidan (hamma qadam avtomatik: Rust `.so` → Gradle → APK):

```bash
make mobileBuild   # app/build/outputs/apk/debug/app-debug.apk
make mobileRun     # build + ulangan telefon/emulyatorga o'rnatish va ishga tushirish
```

`mobileRun` qurilma topilmasa birinchi AVD'ni (yoki `AVD=nom make mobileRun`) ishga tushiradi. Qo'lda yig'ish:

1. Rust kutubxonasini yig'ish (`.so` Gradle ko'radigan `jniLibs` ga yoziladi):

```bash
cd mobile_viewer/core
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -o ../app/src/main/jniLibs build --release
```

Tez tekshirish uchun: `cargo ndk -t arm64-v8a build`.

2. Ilovani yig'ish: `mobile_viewer/` papkasini Android Studio'da oching va Run bosing, yoki

```bash
cd mobile_viewer
gradle wrapper        # bir marta (gradle-wrapper.jar repoda yo'q)
./gradlew assembleDebug
```

APK: `app/build/outputs/apk/debug/app-debug.apk`.

## Ishlatish

1. **Server sozlamalari**: relay server manzili (`host` yoki `host:port`, standart port 7780), server kaliti va qurilma nomi → "Saqlash va qayta ulanish". Tepada "Onlayn" va o'z ID ko'rinadi.
2. Kompyuterning 9 xonali ID sini kiriting (qarovsiz rejim bo'lsa parol ham), sifat shablonini tanlang (desktop viewer'dagi 5 ta shablon + FPS/bitrate/o'lcham slayderlari), "Ulanish".
3. Host tomonda ruxsat berilgach video ochiladi. Birinchi ulanishda Host fingerprint'i tasdiqlanadi.
4. Muvaffaqiyatli ulangan ID'lar "Kontaktlar" ga saqlanadi (onlayn holati ko'rinadi; bosib ID ni to'ldiring, uzoq bosib o'chiring).

### Ko'rish ekrani

- **⌨ Klaviatura** — soft klaviaturani ochadi/yopadi; tagida Esc/Tab/Ctrl/Alt/Win/Del/o'qlar qatori (Ctrl/Alt/Win bir martalik).
  Faqat lotin/ASCII belgilar yuboriladi (protokol evdev klavish kodlari bilan ishlaydi; kirill va boshqa raskladkalar yo'q).
- **Sensor / Touchpad** tugmasi — sichqoncha rejimi (tanlov saqlanadi):
  - *Sensor*: tap = shu nuqtada chap klik, uzoq bosish = o'ng klik, surish = chap tugma bosilgan holda kursor barmoq ostida ketadi, ikki barmoq surish = scroll.
  - *Touchpad*: surish = kursor nisbiy suriladi, tap = chap klik, ikki barmoq tap = o'ng klik, ikki barmoq surish = scroll, tap-tap-surish = drag.
- Ikki barmoq chimchilash = zoom (kursor koordinatalari moslanadi); **1×** zoomni tiklaydi.
- Ekran yopilsa/ilova fonga o'tsa sessiya tugatiladi (v1: faqat oldingi planda ishlaydi).

## Holat (nima tekshirilgan)

- Tekshirilgan: `cargo ndk -t arm64-v8a build` (debug va `--release`) xatosiz; `cargo test -p rdcore` o'tadi.
- **Tekshirilmagan**: Kotlin/Gradle kodi yig'ilmagan (bu muhitda Gradle yo'q edi) va qurilma/emulyatorda sinalmagan:
  MediaCodec dekodlash, SurfaceView zoom (`scaleX/translation`), IME orqali kiritish, tegish gesturelari, argon2 tezligi.
  Birinchi Android Studio build'da kompilyatsiya xatolari chiqishi mumkin.
- Cheklovlar (v1): faqat relay-by-ID (P2P manzil+parol yo'q), fon rejimi yo'q, kirill kiritish yo'q.
