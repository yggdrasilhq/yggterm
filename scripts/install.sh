#!/usr/bin/env bash
set -euo pipefail

REPO="${YGGTERM_REPO:-yggdrasilhq/yggterm}"
API_URL="https://api.github.com/repos/${REPO}/releases/latest"
TMP_DIR="$(mktemp -d)"

log() {
  printf '[yggterm-install] %s\n' "$*" >&2
}

fail() {
  log "$*"
  exit 1
}

cleanup() {
  rm -rf "${TMP_DIR}"
}
trap cleanup EXIT

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    fail "missing required command: $1"
  }
}

need_cmd curl
need_cmd tar
need_cmd uname

os="$(uname -s)"
arch="$(uname -m)"

case "${os}" in
  Linux)
    install_root="${YGGTERM_INSTALL_ROOT:-${HOME}/.local/share/yggterm/direct}"
    ;;
  Darwin)
    install_root="${YGGTERM_INSTALL_ROOT:-${HOME}/Library/Application Support/yggterm/direct}"
    ;;
  *)
    fail "unsupported operating system: ${os}"
    ;;
esac

case "${os}:${arch}" in
  Linux:x86_64|Linux:amd64)
    target_label="linux-x86_64"
    ;;
  Linux:aarch64|Linux:arm64)
    target_label="linux-aarch64"
    ;;
  Darwin:x86_64)
    target_label="macos-x86_64"
    ;;
  Darwin:arm64|Darwin:aarch64)
    target_label="macos-aarch64"
    ;;
  *)
    fail "unsupported architecture: ${arch} on ${os}"
    ;;
esac

log "checking latest release for ${target_label}"
release_json="$(curl -fsSL "${API_URL}")"
release_version="$(
  printf '%s' "${release_json}" \
    | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"v\([^"]*\)".*/\1/p' \
    | head -n1
)"

asset_url() {
  local pattern="$1"
  printf '%s' "${release_json}" \
    | sed 's/\\\\\//\//g' \
    | sed -n "s/.*\"browser_download_url\"[[:space:]]*:[[:space:]]*\"\\([^\"]*${pattern}[^\"]*\\)\".*/\\1/p" \
    | head -n1
}

archive_url="$(asset_url "yggterm-${target_label}\\.tar\\.gz")"
checksum_url="$(asset_url "yggterm-${target_label}\\.tar\\.gz\\.sha256")"

if [[ -z "${release_version}" || -z "${archive_url}" ]]; then
  log "failed to locate a compatible release asset for ${target_label}"
  log "available release assets:"
  printf '%s' "${release_json}" \
    | sed -n 's/.*"name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/  - \1/p' \
    | sed -n '/^  - yggterm/p' >&2
  exit 1
fi

archive_path="${TMP_DIR}/yggterm.tar.gz"
checksum_path="${TMP_DIR}/yggterm.tar.gz.sha256"
log "downloading yggterm ${release_version}"
curl -fL "${archive_url}" -o "${archive_path}"
if [[ -n "${checksum_url}" ]]; then
  curl -fL "${checksum_url}" -o "${checksum_path}"
  log "verifying checksum"
  expected="$(awk '{print $1}' "${checksum_path}")"
  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "${archive_path}" | awk '{print $1}')"
  else
    actual="$(shasum -a 256 "${archive_path}" | awk '{print $1}')"
  fi
  [[ "${expected}" == "${actual}" ]] || {
    fail "checksum verification failed"
  }
fi

version_dir="${install_root}/versions/${release_version}"
mkdir -p "${version_dir}"
tar -xzf "${archive_path}" -C "${TMP_DIR}"

if [[ "${target_label}" == windows-* ]]; then
  binary_name="yggterm-${target_label}.exe"
  installed_binary="${version_dir}/yggterm.exe"
else
  binary_name="yggterm-${target_label}"
  installed_binary="${version_dir}/yggterm"
fi

cp "${TMP_DIR}/${binary_name}" "${installed_binary}"
chmod 0755 "${installed_binary}" || true

cat > "${install_root}/install-state.json" <<JSON
{
  "channel": "direct",
  "repo": "${REPO}",
  "asset_label": "${target_label}",
  "active_version": "${release_version}",
  "active_executable": "${installed_binary}",
  "icon_revision": "${release_version}"
}
JSON

bin_dir="${HOME}/.local/bin"
mkdir -p "${bin_dir}"
ln -sfn "${installed_binary}" "${bin_dir}/yggterm"

log "refreshing desktop integration"
"${installed_binary}" install integrate >/dev/null 2>&1 || true

log "installed yggterm ${release_version}"
log "binary: ${installed_binary}"
if [[ ":${PATH}:" != *":${bin_dir}:"* ]]; then
  log "add ${bin_dir} to PATH if you want the yggterm command in your shell"
fi
