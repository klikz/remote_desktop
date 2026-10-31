#!/usr/bin/env bash
# Android client: `scripts/mobile.sh build` (APK yig'ish) va `scripts/mobile.sh run` (o'rnatish + ishga tushirish).
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

build() {
  command -v cargo-ndk >/dev/null 2>&1 || cargo install cargo-ndk
  rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android >/dev/null
  # Emulyator uchun x86_64 ham kerak
  (cd "$MV/core" && cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -o ../app/src/main/jniLibs build --release)
  (cd "$MV" && ./gradlew --no-daemon assembleDebug)
  echo "APK tayyor: $APK"
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
  *) echo "foydalanish: $0 build|run"; exit 1 ;;
esac
