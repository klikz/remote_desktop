# Ubuntu: `make` GitHub dan yangilanishlarni oladi, yangi versiyani yig'adi va ishga tushiradi.
#   make          yangilash + build + ishga tushirish
#   make update   faqat git pull
#   make build    faqat build (pull qilmasdan)
#   make run      pull qilmasdan, mavjud build ni ishga tushirish
#   make clean    build fayllarini o'chirish
#   NO_HOST=1 make   GStreamer dev paketlarisiz (faqat Viewer)

VIEWER := target/release/viewer

.PHONY: all update build run clean

all: update build run

update:
	git pull --ff-only

build:
	bash scripts/build-viewer.sh

run:
	@test -x $(VIEWER) || { echo "$(VIEWER) topilmadi: avval make build"; exit 1; }
	$(VIEWER)

clean:
	cargo clean
