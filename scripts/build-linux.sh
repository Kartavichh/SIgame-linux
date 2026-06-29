#!/usr/bin/env bash
# Сборка релизных артефактов SIGame-RS под Linux.
#
# Результат:
#   target/release/sigame-server              — headless-сервер (один файл)
#   target/release/bundle/deb/*.deb           — клиент для Debian/Ubuntu
#   target/release/bundle/appimage/*.AppImage — клиент одним файлом (если собрался)
#
# Запуск из корня репозитория:  ./scripts/build-linux.sh
set -euo pipefail

cd "$(dirname "$0")/.."

echo "==> Сборка сервера + подготовка sidecar для клиента…"
# Собирает sigame-server и кладёт его как sidecar (binaries/sigame-server-<triple>),
# чтобы Tauri вшил сервер в .deb/AppImage (externalBin). Файл нужен ещё и на этапе
# компиляции клиента, поэтому делаем это ДО сборки клиента.
./scripts/prep-sidecar.sh

echo "==> Сборка клиента (Tauri: .deb + AppImage)…"
# Нужны системные зависимости Tauri (см. README). AppImage подкачивает linuxdeploy.
if command -v cargo-tauri >/dev/null 2>&1 || cargo tauri --version >/dev/null 2>&1; then
  cargo tauri build --bundles deb appimage
else
  echo "!! cargo-tauri не найден. Установите:  cargo install tauri-cli --version '^2'"
  echo "   Сервер собран, клиент пропущен."
fi

echo
echo "Готово. Артефакты:"
echo "  сервер: target/release/sigame-server"
echo "  клиент: target/release/bundle/"
