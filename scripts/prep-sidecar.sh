#!/usr/bin/env bash
# Готовит sigame-server как sidecar для встраивания в клиент (Tauri externalBin).
#
# Tauri ждёт файл с суффиксом целевого триплета рядом с tauri.conf.json:
#   client/src-tauri/binaries/sigame-server-<triple>
# Этот файл нужен ЛЮБОЙ сборке клиента (tauri-build проверяет его наличие при
# компиляции) — и для `cargo tauri build`, и для `cargo tauri dev`, и даже для
# `cargo build -p sigame-client`. Поэтому запускайте скрипт перед сборкой клиента.
set -euo pipefail

cd "$(dirname "$0")/.."

TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
if [ -z "$TRIPLE" ]; then
  echo "!! не удалось определить целевой триплет (rustc -vV)"; exit 1
fi

echo "==> Сборка sigame-server (release)…"
cargo build --release -p sigame-server

mkdir -p client/src-tauri/binaries
cp -f target/release/sigame-server "client/src-tauri/binaries/sigame-server-${TRIPLE}"

echo "==> Sidecar готов: client/src-tauri/binaries/sigame-server-${TRIPLE}"
