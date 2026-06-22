#!/usr/bin/env bash
set -euo pipefail

REPO="${DOCKERCTL_REPO:-badwichell007/dockerctl}"
BIN_NAME="dockerctl"
DEST_DIR="${DOCKERCTL_INSTALL_DIR:-${XDG_BIN_HOME:-$HOME/.local/bin}}"
VERSION="${DOCKERCTL_VERSION:-latest}"
TMP_DIR="$(mktemp -d)"

cleanup() {
  rm -rf "${TMP_DIR}"
}
trap cleanup EXIT

detect_target() {
  local os arch
  os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  arch="$(uname -m)"
  case "${os}" in
    linux) os="unknown-linux-gnu" ;;
    *) echo "不支持的系统: ${os}" >&2; exit 1 ;;
  esac
  case "${arch}" in
    x86_64|amd64) arch="x86_64" ;;
    aarch64|arm64) arch="aarch64" ;;
    *) echo "不支持的架构: ${arch}" >&2; exit 1 ;;
  esac
  printf "%s-%s" "${arch}" "${os}"
}

download_release() {
  local target archive base_url version_path
  target="$(detect_target)"
  archive="${BIN_NAME}-${target}.tar.gz"
  if [[ "${VERSION}" == "latest" ]]; then
    version_path="latest/download"
  else
    version_path="download/${VERSION}"
  fi
  base_url="https://github.com/${REPO}/releases/${version_path}/${archive}"

  echo "下载 ${base_url}"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "${base_url}" -o "${TMP_DIR}/${archive}"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "${TMP_DIR}/${archive}" "${base_url}"
  else
    return 1
  fi
  tar -xzf "${TMP_DIR}/${archive}" -C "${TMP_DIR}"
  install_binary "${TMP_DIR}/${BIN_NAME}"
}

build_from_source() {
  if ! command -v cargo >/dev/null 2>&1; then
    echo "未找到预编译包且本机没有 cargo，无法源码安装。" >&2
    echo "请安装 Rust: https://rustup.rs/" >&2
    exit 1
  fi

  echo "预编译包不可用，尝试 cargo install 源码安装..."
  cargo install --git "https://github.com/${REPO}.git" --locked --bin "${BIN_NAME}" --root "${TMP_DIR}/cargo-root"
  install_binary "${TMP_DIR}/cargo-root/bin/${BIN_NAME}"
}

install_binary() {
  local src="$1"
  mkdir -p "${DEST_DIR}"
  install -m 755 "${src}" "${DEST_DIR}/${BIN_NAME}"
}

main() {
  if ! download_release; then
    build_from_source
  fi

  echo "安装完成: ${DEST_DIR}/${BIN_NAME}"
  if [[ ":${PATH}:" != *":${DEST_DIR}:"* ]]; then
    echo
    echo "注意: ${DEST_DIR} 不在 PATH。请追加："
    echo "export PATH=\"${DEST_DIR}:\$PATH\""
  fi
  echo
  echo "运行: ${BIN_NAME}"
}

main "$@"
