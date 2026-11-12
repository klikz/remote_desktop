#!/usr/bin/env bash
# Versiyani avtomatik oshiradi (patch): 0.1.0 -> 0.1.1 -> 0.1.2 ...
# Manba: Cargo.toml [workspace.package] version. Android (versionName/versionCode) ham shunga moslanadi.
# Yangi minor/major uchun Cargo.toml dagi versiyani qo'lda o'zgartiring (masalan 0.2.0 yoki 1.0.0).
#   scripts/bump-version.sh          oshiradi va yangi versiyani chiqaradi
#   scripts/bump-version.sh --show   faqat joriy versiyani chiqaradi
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CARGO="$ROOT/Cargo.toml"
GRADLE="$ROOT/mobile_viewer/app/build.gradle.kts"
[ -f "$GRADLE" ] || GRADLE="$ROOT/mobile_viewer/app/build.gradle"

cur="$(sed -n '/^\[workspace.package\]/,/^\[/{s/^version *= *"\([0-9]*\.[0-9]*\.[0-9]*\)".*/\1/p}' "$CARGO" | head -1)"
[ -n "$cur" ] || { echo "Cargo.toml da versiya topilmadi"; exit 1; }
if [ "${1:-}" = "--show" ]; then echo "$cur"; exit 0; fi

IFS=. read -r MA MI PA <<<"$cur"
new="$MA.$MI.$((PA + 1))"
code=$((MA * 10000 + MI * 100 + PA + 1))   # Android versionCode: 0.1.5 -> 105 (har doim o'sib boradi)

sed -i "/^\[workspace.package\]/,/^\[/s/^version *= *\"$cur\"/version = \"$new\"/" "$CARGO"
if [ -f "$GRADLE" ]; then
  sed -i -E "s/versionName *= *\"[^\"]*\"/versionName = \"$new\"/; s/versionCode *= *[0-9]+/versionCode = $code/" "$GRADLE"
fi
mkdir -p "$ROOT/builds" && echo "$new" > "$ROOT/builds/VERSION"
echo "$new"
