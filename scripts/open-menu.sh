#!/usr/bin/env bash
set -euo pipefail

PROJECT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_PATH="$(command -v dockerctl || true)"

if [[ -z "${BIN_PATH}" ]]; then
  BIN_PATH="${PROJECT_DIR}/target/release/dockerctl"
fi

if [[ ! -x "${BIN_PATH}" ]]; then
  echo "未找到可执行文件: ${BIN_PATH}"
  echo "请先执行: cd \"${PROJECT_DIR}\" && cargo build --release"
  exit 1
fi

if command -v "foot" >/dev/null 2>&1; then
  exec foot -e "${BIN_PATH}" "$@"
fi
if command -v "alacritty" >/dev/null 2>&1; then
  exec alacritty -e "${BIN_PATH}" "$@"
fi
if command -v "kitty" >/dev/null 2>&1; then
  exec kitty "${BIN_PATH}" "$@"
fi
if command -v "wezterm" >/dev/null 2>&1; then
  exec wezterm start -- "${BIN_PATH}" "$@"
fi

echo "未找到可用终端，请安装 foot/alacritty/kitty/wezterm 中任意一个。"
exit 1
