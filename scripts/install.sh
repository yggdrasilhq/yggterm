#!/usr/bin/env bash
set -euo pipefail

REPO="${YGGTERM_REPO:-yggdrasilhq/yggterm}"
API_URL="https://api.github.com/repos/${REPO}/releases/latest"
INSTALL_ROOT="${YGGTERM_INSTALL_ROOT:-${HOME}/.local}"
BIN_DIR="${INSTALL_ROOT}/bin"
LIB_DIR="${INSTALL_ROOT}/lib/yggterm"
TMP_DIR="$(mktemp -d)"

cleanup() {
  rm -rf "${TMP_DIR}"
}
trap cleanup EXIT

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 1
  }
}

need_cmd curl
need_cmd uname
need_cmd tar

os="$(uname -s)"
arch="$(uname -m)"

case "${os}" in
  Linux) ;;
  *)
    echo "unsupported operating system: ${os}" >&2
    exit 1
    ;;
esac

case "${arch}" in
  x86_64|amd64)
    target_label="linux-x86_64"
    deb_arch="amd64"
    ;;
  *)
    echo "unsupported architecture: ${arch}" >&2
    exit 1
    ;;
esac

release_json="$(curl -fsSL "${API_URL}")"

asset_url() {
  local pattern="$1"
  printf '%s' "${release_json}" | sed 's/\\\\\//\//g' | grep -o "\"browser_download_url\":\"[^\"]*${pattern}[^\"]*\"" | head -n1 | cut -d'"' -f4
}

deb_url="$(asset_url "yggterm_.*_${deb_arch}\\.deb")"
tar_url="$(asset_url "yggterm-${target_label}\\.tar\\.gz")"

if command -v dpkg >/dev/null 2>&1 && command -v sudo >/dev/null 2>&1 && [[ -n "${deb_url}" ]]; then
  deb_path="${TMP_DIR}/yggterm.deb"
  curl -fL "${deb_url}" -o "${deb_path}"
  sudo dpkg -i "${deb_path}"
  echo "installed yggterm via .deb"
  exit 0
fi

if [[ -z "${tar_url}" ]]; then
  echo "failed to locate a compatible release asset for ${target_label}" >&2
  exit 1
fi

mkdir -p "${BIN_DIR}" "${LIB_DIR}"
archive_path="${TMP_DIR}/yggterm.tar.gz"
curl -fL "${tar_url}" -o "${archive_path}"
tar -xzf "${archive_path}" -C "${TMP_DIR}"

cp "${TMP_DIR}/yggterm-${target_label}" "${BIN_DIR}/yggterm"
chmod 0755 "${BIN_DIR}/yggterm"

if [[ ":${PATH}:" != *":${BIN_DIR}:"* ]]; then
  echo "installed to ${BIN_DIR}/yggterm"
  echo "add ${BIN_DIR} to PATH if needed"
else
  echo "installed to ${BIN_DIR}/yggterm"
fi
