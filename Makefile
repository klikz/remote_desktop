# Ubuntu: `make` GitHub dan yangilanishlarni oladi, yangi versiyani yig'adi va ishga tushiradi.
#   make          yangilash + build + ishga tushirish
#   make update   faqat git pull
#   make build    faqat build (pull qilmasdan)
#   make run      build (o'zgargan bo'lsa) va ishga tushirish, pull qilmasdan
#   make push     hamma o'zgarishni commit (xabar ".") va push qilish; xabar: make push MSG="matn"
#   make clean    build fayllarini o'chirish
#   NO_HOST=1 make   GStreamer dev paketlarisiz (faqat Viewer)

VIEWER := target/release/viewer

MSG ?= .

.PHONY: all update build run push clean

all: update build run

update:
	git pull --ff-only

build:
	bash scripts/build-viewer.sh

run: build
	@test -x $(VIEWER) || { echo "$(VIEWER) topilmadi"; exit 1; }
	$(VIEWER)

push:
	git add -A
	@git diff --cached --quiet || git commit -m "$(MSG)"
	git push

clean:
	cargo clean
