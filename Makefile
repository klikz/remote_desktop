# Ubuntu: `make` GitHub dan yangilanishlarni oladi, yangi versiyani yig'adi va ishga tushiradi.
#   make          yangilash + build + ishga tushirish
#   make update   faqat git pull
#   make build    faqat build (pull qilmasdan)
#   make run      build, o'rnatilgan Host xizmatini yangilash (kerak bo'lsa) va Viewer ni ishga tushirish
#   make push     hamma o'zgarishni commit (xabar ".") va push qilish; xabar: make push MSG="matn"
#   make buildWin Windows uchun build (release)
#   Tayyor fayllar hammasi builds/ papkasiga tushadi (PC va Android)
#   make runWin   Windows: cargo run -p viewer (debug, terminalda loglar ko'rinadi)
#   make mobileBuild  Android APK yig'ish (mobile_viewer/app/build/outputs/apk/debug/app-debug.apk)
#   make mobileRun    APK yig'ish, telefon/emulyatorga o'rnatish va ishga tushirish
#   make mobileKeystore  Android release imzo kalitini yaratish (bir marta)
#   make mobileRelease   imzolangan release APK va AAB
#   make clean    build fayllarini o'chirish
#   NO_HOST=1 make   GStreamer dev paketlarisiz (faqat Viewer)

VIEWER := target/release/viewer

# apt dagi cargo eski (Cargo.lock v4 ni o'qiy olmaydi): rustup dagi yangi Rust ishlatiladi
export PATH := $(HOME)/.cargo/bin:$(PATH)

MSG ?= .

# Windows PowerShell'da `bash` WSL'ga tushadi (distro yo'q bo'lsa xato): Git Bash ishlatiladi
ifeq ($(OS),Windows_NT)
MOBILE_BASH ?= "C:/Program Files/Git/bin/bash.exe"
else
MOBILE_BASH ?= bash
endif

.PHONY: all update toolchain build install-host run push clean buildWin runWin mobileBuild mobileRun mobileKeystore mobileRelease

all: update build run

update:
	git pull --ff-only

toolchain:
	@test -x $(HOME)/.cargo/bin/cargo || { echo "rustup o'rnatilmoqda..."; curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal; }

build: toolchain
	bash scripts/build-viewer.sh
	@mkdir -p builds && cp -f $(VIEWER) builds/msremote-viewer-linux
	@echo "Tayyor: builds/msremote-viewer-linux"

HOST_BIN := $(HOME)/.local/bin/remote-desktop-host

install-host: build
	@if [ -f target/release/host-linux ] && systemctl --user is-enabled remote-desktop-host >/dev/null 2>&1 \
	   && ! cmp -s target/release/host-linux $(HOST_BIN); then \
	  echo "Host yangilanmoqda..."; \
	  install -m 755 target/release/host-linux $(HOST_BIN).new && mv -f $(HOST_BIN).new $(HOST_BIN) \
	  && systemctl --user restart remote-desktop-host; \
	fi

run: install-host
	@test -x $(VIEWER) || { echo "$(VIEWER) topilmadi"; exit 1; }
	$(VIEWER)

push:
	git add -A
	@git diff --cached --quiet || git commit -m "$(MSG)"
	git push

clean:
	cargo clean

# --- Windows (Git Bash / make bilan) -------------------------------------------------------------

buildWin:
	cargo build --release -p viewer
	@$(MOBILE_BASH) -c "mkdir -p builds && cp -f target/release/viewer.exe builds/msremote-viewer.exe && echo Tayyor: builds/msremote-viewer.exe"

runWin:
	cargo run -p viewer

# --- Android (mobile_viewer/) --------------------------------------------------------------------

mobileBuild:
	$(MOBILE_BASH) scripts/mobile.sh build

mobileRun:
	$(MOBILE_BASH) scripts/mobile.sh run

mobileKeystore:
	$(MOBILE_BASH) scripts/mobile.sh keystore

mobileRelease:
	$(MOBILE_BASH) scripts/mobile.sh release
