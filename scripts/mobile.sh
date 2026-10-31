#!/usr/bin/env bash
# Android client:
#   scripts/mobile.sh build      debug APK
#   scripts/mobile.sh run        debug APK + telefon/emulyatorga o'rnatish va ishga tushirish
#   scripts/mobile.sh keystore   release imzo kaliti (mobile_viewer/release.jks + keystore.properties)
#   scripts/mobile.sh release    imzolangan release APK va AAB (Google Play uchun)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MV="$ROOT/mobile_viewer"
APK="$MV/app/build/outputs/apk/debug/app-debug.apk"
PKG=com.remotedesktop.mobile

export PATH="$HOME/.cargo/bin:$PATH"
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/AppData/Local/Android/Sdk}}"
[ -d "$SDK" ] || SDK="$HOME/Android/Sdk"
[ -d "$SDK" ] || { echo "Android SDK topilmadi (ANDROID_HOME ni o'rnating)"; exit 1; }
export ANDROID_HOME="$SDK"

# NDK: ANDROID_NDK_HOME yoki SDK ichidagi eng yangi 27.x/boshqa versiya
if [ -z "${ANDROID_NDK_HOME:-}" ]; then
  ANDROID_NDK_HOME="$(ls -d "$SDK"/ndk/27.* 2>/dev/null | sort -V | tail -1 || true)"
  [ -n "$ANDROID_NDK_HOME" ] || ANDROID_NDK_HOME="$(ls -d "$SDK"/ndk/* 2>/dev/null | sort -V | tail -1 || true)"
  [ -n "$ANDROID_NDK_HOME" ] || { echo "NDK topilmadi (SDK Manager'dan o'rnating)"; exit 1; }
  export ANDROID_NDK_HOME
fi

# JDK: Android Studio bilan keladigan jbr
if ! command -v java >/dev/null 2>&1; then
  for j in "/c/Program Files/Android/Android Studio/jbr" "$HOME/android-studio/jbr" "/opt/android-studio/jbr"; do
    [ -x "$j/bin/java" ] && export JAVA_HOME="$j" PATH="$j/bin:$PATH" && break
  done
fi
command -v java >/dev/null 2>&1 || [ -n "${JAVA_HOME:-}" ] || { echo "JDK 17 topilmadi (JAVA_HOME ni o'rnating)"; exit 1; }

# Windows (Git Bash): Gradle/Java uchun yo'llar C:/... ko'rinishida bo'lishi kerak
if command -v cygpath >/dev/null 2>&1; then
  export ANDROID_HOME="$(cygpath -m "$ANDROID_HOME")"
  export ANDROID_NDK_HOME="$(cygpath -m "$ANDROID_NDK_HOME")"
  [ -z "${JAVA_HOME:-}" ] || export JAVA_HOME="$(cygpath -m "$JAVA_HOME")"
fi

native() {
  command -v cargo-ndk >/dev/null 2>&1 || cargo install cargo-ndk
  rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android >/dev/null
  # Emulyator uchun x86_64 ham kerak
  (cd "$MV/core" && cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -o ../app/src/main/jniLibs build --release)
}

build() {
  native
  (cd "$MV" && ./gradlew --no-daemon assembleDebug)
  mkdir -p "$ROOT/builds" && cp -f "$APK" "$ROOT/builds/msremote-debug.apk"
  echo "APK tayyor: $ROOT/builds/msremote-debug.apk"
}

keytool_bin() {
  if [ -n "${JAVA_HOME:-}" ] && [ -x "$JAVA_HOME/bin/keytool" ]; then echo "$JAVA_HOME/bin/keytool"
  elif [ -n "${JAVA_HOME:-}" ] && [ -x "$JAVA_HOME/bin/keytool.exe" ]; then echo "$JAVA_HOME/bin/keytool.exe"
  else command -v keytool; fi
}

# Release imzo kaliti: bir marta yaratiladi. Kalit faylini va parolni yo'qotsangiz, Play'dagi ilovani yangilab bo'lmaydi.
keystore() {
  local jks="$MV/release.jks" props="$MV/keystore.properties"
  if [ -e "$jks" ] || [ -e "$props" ]; then
    echo "Kalit allaqachon bor: $jks (yangisini yaratish uchun avval eskisini o'zingiz ko'chiring yoki o'chiring)"
    exit 1
  fi
  local pw pw2
  read -r -s -p "Kalit ombori paroli (kamida 6 belgi): " pw; echo
  read -r -s -p "Parolni takrorlang: " pw2; echo
  [ "$pw" = "$pw2" ] || { echo "Parollar mos kelmadi"; exit 1; }
  [ "${#pw}" -ge 6 ] || { echo "Parol juda qisqa"; exit 1; }
  local cn="${KEY_CN:-MS Remote}"
  RD_KS_PW="$pw" "$(keytool_bin)" -genkeypair -keystore "$jks" -alias msremote -keyalg RSA -keysize 4096 -validity 10000     -dname "CN=$cn" -storepass:env RD_KS_PW -keypass:env RD_KS_PW
  umask 077
  printf 'storeFile=release.jks
storePassword=%s
keyAlias=msremote
keyPassword=%s
' "$pw" "$pw" > "$props"
  echo "Tayyor: $jks va $props (ikkalasi ham gitga qo'shilmaydi)."
  echo "MUHIM: release.jks va parolni xavfsiz joyda zaxiralang — ularsiz ilovaning yangi versiyasini imzolab bo'lmaydi."
}

release() {
  if [ ! -f "$MV/keystore.properties" ] && [ -z "${RD_KEYSTORE:-}" ]; then
    echo "Imzo kaliti yo'q. Avval: make mobileKeystore  (yoki RD_KEYSTORE, RD_KEYSTORE_PASSWORD, RD_KEY_ALIAS, RD_KEY_PASSWORD)"
    exit 1
  fi
  native
  (cd "$MV" && ./gradlew --no-daemon assembleRelease bundleRelease)
  local apk="$MV/app/build/outputs/apk/release/app-release.apk" aab="$MV/app/build/outputs/bundle/release/app-release.aab"
  [ -f "$apk" ] || { echo "Imzolangan APK chiqmadi (kalit sozlamalarini tekshiring)"; exit 1; }
  local signer
  signer="$(ls -d "$SDK"/build-tools/* 2>/dev/null | sort -V | tail -1)/apksigner"
  [ -e "$signer" ] || signer="$signer.bat"
  if [ -e "$signer" ]; then "$signer" verify --print-certs "$apk" | grep -E "certificate (DN|SHA-256)" | sort -u || true; fi
  mkdir -p "$ROOT/builds" && cp -f "$apk" "$ROOT/builds/msremote-release.apk" && cp -f "$aab" "$ROOT/builds/msremote-release.aab"
  echo "APK (to'g'ridan-to'g'ri o'rnatish uchun): $ROOT/builds/msremote-release.apk"
  echo "AAB (Google Play uchun):                  $ROOT/builds/msremote-release.aab"
}


run() {
  build
  ADB="$SDK/platform-tools/adb"; [ -x "$ADB" ] || ADB="$SDK/platform-tools/adb.exe"
  "$ADB" start-server >/dev/null
  if ! "$ADB" devices | awk 'NR>1 && $2=="device"' | grep -q .; then
    EMU="$SDK/emulator/emulator"; [ -x "$EMU" ] || EMU="$SDK/emulator/emulator.exe"
    AVD="${AVD:-$("$EMU" -list-avds | head -1)}"
    [ -n "$AVD" ] || { echo "Qurilma ulanmagan va AVD yo'q: telefonni USB bilan ulang (USB debugging) yoki AVD yarating"; exit 1; }
    echo "Emulyator ishga tushmoqda: $AVD"
    nohup "$EMU" -avd "$AVD" >/dev/null 2>&1 &
    disown
    "$ADB" wait-for-device
    until [ "$("$ADB" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = "1" ]; do sleep 2; done
  fi
  "$ADB" install -r "$APK"
  "$ADB" shell am start -n "$PKG/.MainActivity"
}

case "${1:-}" in
  build) build ;;
  run) run ;;
  keystore) keystore ;;
  release) release ;;
  *) echo "foydalanish: $0 build|run|keystore|release"; exit 1 ;;
esac
