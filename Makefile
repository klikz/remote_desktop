# Ubuntu: `make` GitHub dan yangilanishlarni oladi, yangi versiyani yig'adi va ishga tushiradi.
#   make          yangilash + build + ishga tushirish
#   make update   faqat git pull
#   make build    faqat build (pull qilmasdan)
#   make run      build, o'rnatilgan Host xizmatini yangilash (kerak bo'lsa) va Viewer ni ishga tushirish
#   make push     hamma o'zgarishni commit (xabar ".") va push qilish; xabar: make push MSG="matn"
#   make clean    build fayllarini o'chirish
#   NO_HOST=1 make   GStreamer dev paketlarisiz (faqat Viewer)

VIEWER := target/release/viewer

# apt dagi cargo eski (Cargo.lock v4 ni o'qiy olmaydi): rustup dagi yangi Rust ishlatiladi
export PATH := $(HOME)/.cargo/bin:$(PATH)

MSG ?= .

.PHONY: all update toolchain build install-host run push clean

all: update build run

update:
	git pull --ff-only

toolchain:
	@test -x $(HOME)/.cargo/bin/cargo || { echo "rustup o'rnatilmoqda..."; curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal; }

build: toolchain
	bash scripts/build-viewer.sh

install-host: build
	@if [ -f target/release/host-linux ] && systemctl --user is-enabled remote-desktop-host >/dev/null 2>&1; then \n	  if ! cmp -s target/release/host-linux $(HOME)/.local/bin/remote-desktop-host; then \n	    echo "Host yangilanmoqda..."; \n	    install -m 755 target/release/host-linux $(HOME)/.local/bin/remote-desktop-host.new && \n	    mv -f $(HOME)/.local/bin/remote-desktop-host.new $(HOME)/.local/bin/remote-desktop-host && \n	    systemctl --user restart remote-desktop-host; \n	  fi; \n	fi

run: install-host
	@test -x $(VIEWER) || { echo "$(VIEWER) topilmadi"; exit 1; }
	$(VIEWER)

push:
	git add -A
	@git diff --cached --quiet || git commit -m "$(MSG)"
	git push

clean:
	cargo clean
