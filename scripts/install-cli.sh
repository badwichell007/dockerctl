#!/usr/bin/env bash
set -euo pipefail

PROJECT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
DEST_DIR="${1:-${XDG_BIN_HOME:-$HOME/.local/bin}}"
BIN_NAME="dockerctl"
SRC_BIN="${PROJECT_DIR}/target/release/${BIN_NAME}"
DST_BIN="${DEST_DIR}/${BIN_NAME}"

if [[ ! -x "${SRC_BIN}" ]]; then
  echo "正在构建 release 二进制..."
  cargo build --release --manifest-path "${PROJECT_DIR}/Cargo.toml"
fi

mkdir -p "${DEST_DIR}"
install -m 755 "${SRC_BIN}" "${DST_BIN}"

echo "安装完成: ${DST_BIN}"
echo "现在可以直接执行: ${BIN_NAME}"
echo
echo "常用命令:"
echo "  ${BIN_NAME}                 # 进入 TUI"
echo "  ${BIN_NAME} list            # 列出项目"
echo "  ${BIN_NAME} doctor          # 诊断异常"
echo "  ${BIN_NAME} plan purge app  # 完全删除前风险预演"
echo
echo "Shell 补全示例:"
echo "  ${BIN_NAME} completion bash > ~/.local/share/bash-completion/completions/${BIN_NAME}"
echo "  ${BIN_NAME} completion zsh  > ~/.zfunc/_${BIN_NAME}"
echo "  ${BIN_NAME} completion fish > ~/.config/fish/completions/${BIN_NAME}.fish"

if [[ ":${PATH}:" != *":${DEST_DIR}:"* ]]; then
  echo
  echo "注意: ${DEST_DIR} 不在当前 PATH。"
  echo "请在 ~/.zshrc 追加："
  echo "export PATH=\"${DEST_DIR}:\$PATH\""
fi
