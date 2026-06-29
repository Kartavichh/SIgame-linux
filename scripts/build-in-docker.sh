#!/usr/bin/env bash
# Собирает SIGame-RS в контейнере Ubuntu 22.04, чтобы бинари работали на старых
# системах (glibc 2.35+), а не только на машине разработчика (Ubuntu 24.04).
#
# Требуется: установленный Docker и доступ в интернет (тянет образ и зависимости).
# Запуск из корня репозитория:  ./scripts/build-in-docker.sh
#
# Результат (как у обычной сборки, но совместимый со старым glibc):
#   target/release/sigame-server
#   target/release/bundle/deb/*.deb
#   target/release/bundle/appimage/*.AppImage
set -euo pipefail

cd "$(dirname "$0")/.."

echo "==> Сборка образа сборщика (Ubuntu 22.04)…"
docker build -t sigame-build -f scripts/Dockerfile.build .

echo "==> Сборка артефактов в контейнере…"
# APPIMAGE_EXTRACT_AND_RUN=1 — собрать AppImage без FUSE (внутри контейнера FUSE нет).
# В конце возвращаем владельца target обратно текущему пользователю (внутри — root).
docker run --rm \
  -e APPIMAGE_EXTRACT_AND_RUN=1 \
  -v "$PWD":/src \
  sigame-build \
  bash -c "scripts/build-linux.sh && chown -R $(id -u):$(id -g) target"

echo
echo "Готово. Совместимые со старым glibc артефакты:"
echo "  сервер: target/release/sigame-server"
echo "  клиент: target/release/bundle/"
echo
echo "Проверить минимальный glibc клиента:"
echo "  objdump -T target/release/sigame-client | grep -oE 'GLIBC_[0-9.]+' | sort -V | tail -1"
