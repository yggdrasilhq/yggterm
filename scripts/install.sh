#!/usr/bin/env sh
# Keep this script POSIX-sh compatible because the documented install flow is:
# curl -fsSL .../install.sh | sh
set -eu
(set -o pipefail) >/dev/null 2>&1 && set -o pipefail

REPO="${YGGTERM_REPO:-yggdrasilhq/yggterm}"
LATEST_URL="https://github.com/${REPO}/releases/latest"
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
need_cmd sed

write_launcher_wrapper() {
  bin_dir="${HOME}/.local/bin"
  launcher_path="${bin_dir}/yggterm"
  mkdir -p "${bin_dir}"
  cat > "${launcher_path}" <<EOF
#!/usr/bin/env sh
# yggterm-direct-launcher-v2
set -eu
ROOT='${install_root}'
STATE="\$ROOT/install-state.json"
target=""
if [ -f "\$STATE" ]; then
  target="\$(sed -n 's/.*"active_executable"[[:space:]]*:[[:space:]]*"\\([^"]*\\)".*/\\1/p' "\$STATE" | head -n1)"
fi
if [ -z "\$target" ] || [ ! -x "\$target" ]; then
  latest_dir="\$(ls -td "\$ROOT"/versions/* 2>/dev/null | head -n 1 || true)"
  if [ -n "\$latest_dir" ] && [ -x "\$latest_dir/yggterm" ]; then
    target="\$latest_dir/yggterm"
  fi
fi
if [ -z "\$target" ] || [ ! -x "\$target" ]; then
  target='${installed_binary}'
fi
[ -x "\$target" ] || {
  printf '%s\n' 'yggterm launcher: no runnable executable found' >&2
  exit 1
}
if [ "${YGGTERM_ENABLE_ACCESSIBILITY:-0}" != "1" ] && [ -z "${NO_AT_BRIDGE+x}" ]; then
  export NO_AT_BRIDGE=1
fi
if [ "${YGGTERM_ENABLE_WEBKIT_COMPOSITING:-0}" != "1" ] && [ -z "${WEBKIT_DISABLE_COMPOSITING_MODE+x}" ]; then
  export WEBKIT_DISABLE_COMPOSITING_MODE=1
fi
export YGGTERM_DIRECT_INSTALL_ROOT='${install_root}'
exec "\$target" "\$@"
EOF
  chmod 0755 "${launcher_path}" || true
}

write_mock_cli_wrapper() {
  bin_dir="${HOME}/.local/bin"
  launcher_path="${bin_dir}/yggterm-mock-cli"
  mkdir -p "${bin_dir}"
  cat > "${launcher_path}" <<EOF
#!/usr/bin/env sh
# yggterm-direct-launcher-v2
set -eu
ROOT='${install_root}'
target=""
latest_dir="\$(ls -td "\$ROOT"/versions/* 2>/dev/null | head -n 1 || true)"
if [ -n "\$latest_dir" ] && [ -x "\$latest_dir/yggterm-mock-cli" ]; then
  target="\$latest_dir/yggterm-mock-cli"
fi
if [ -z "\$target" ] || [ ! -x "\$target" ]; then
  target='${installed_mock_cli}'
fi
[ -x "\$target" ] || {
  printf '%s\n' 'yggterm-mock-cli launcher: no runnable executable found' >&2
  exit 1
}
if [ "${YGGTERM_ENABLE_ACCESSIBILITY:-0}" != "1" ] && [ -z "${NO_AT_BRIDGE+x}" ]; then
  export NO_AT_BRIDGE=1
fi
if [ "${YGGTERM_ENABLE_WEBKIT_COMPOSITING:-0}" != "1" ] && [ -z "${WEBKIT_DISABLE_COMPOSITING_MODE+x}" ]; then
  export WEBKIT_DISABLE_COMPOSITING_MODE=1
fi
export YGGTERM_DIRECT_INSTALL_ROOT='${install_root}'
exec "\$target" "\$@"
EOF
  chmod 0755 "${launcher_path}" || true
}

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
latest_effective_url="$(curl -fsSL -o /dev/null -w '%{url_effective}' "${LATEST_URL}")"
release_tag="$(printf '%s\n' "${latest_effective_url}" | sed -n 's#.*/tag/\(v[^/?#]*\).*#\1#p' | tail -n1)"
release_version="$(printf '%s' "${release_tag}" | sed 's/^v//')"
archive_url="https://github.com/${REPO}/releases/download/${release_tag}/yggterm-${target_label}.tar.gz"
checksum_url="${archive_url}.sha256"

[ -n "${release_tag}" ] || fail "failed to resolve latest release tag from ${LATEST_URL}"
[ -n "${release_version}" ] || fail "failed to resolve latest release version from ${release_tag}"

current_version=""
state_path="${install_root}/install-state.json"
if [ -f "${state_path}" ]; then
  current_version="$(
    sed -n 's/.*"active_version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "${state_path}" | head -n1
  )"
fi

if [ -n "${current_version}" ] && [ "${current_version}" = "${release_version}" ]; then
  log "yggterm ${release_version} is already installed"
  current_binary="$(
    sed -n 's/.*"active_executable"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "${state_path}" | head -n1
  )"
  if [ -n "${current_binary}" ] && [ -x "${current_binary}" ]; then
    installed_binary="${current_binary}"
    installed_mock_cli="$(dirname "${current_binary}")/yggterm-mock-cli"
    write_launcher_wrapper
    write_mock_cli_wrapper
    log "refreshing desktop integration"
    "${current_binary}" install integrate >/dev/null 2>&1 || true
    log "binary: ${current_binary}"
  fi
  exit 0
fi

if [ -n "${current_version}" ]; then
  log "updating yggterm ${current_version} -> ${release_version}"
else
  log "installing yggterm ${release_version}"
fi

archive_path="${TMP_DIR}/yggterm.tar.gz"
checksum_path="${TMP_DIR}/yggterm.tar.gz.sha256"
log "downloading yggterm ${release_version}"
curl -fL "${archive_url}" -o "${archive_path}"
if [ -n "${checksum_url}" ]; then
  curl -fL "${checksum_url}" -o "${checksum_path}"
  log "verifying checksum"
  expected="$(awk '{print $1}' "${checksum_path}")"
  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "${archive_path}" | awk '{print $1}')"
  else
    actual="$(shasum -a 256 "${archive_path}" | awk '{print $1}')"
  fi
  [ "${expected}" = "${actual}" ] || {
    fail "checksum verification failed"
  }
fi

version_dir="${install_root}/versions/${release_version}"
mkdir -p "${version_dir}"
tar -xzf "${archive_path}" -C "${TMP_DIR}"

case "${target_label}" in
  windows-*)
  binary_name="yggterm-${target_label}.exe"
  headless_binary_name="yggterm-headless-${target_label}.exe"
  mock_cli_binary_name="yggterm-mock-cli-${target_label}.exe"
  installed_binary="${version_dir}/yggterm.exe"
  installed_headless_binary="${version_dir}/yggterm-headless.exe"
  installed_mock_cli="${version_dir}/yggterm-mock-cli.exe"
  ;;
  *)
  binary_name="yggterm-${target_label}"
  headless_binary_name="yggterm-headless-${target_label}"
  mock_cli_binary_name="yggterm-mock-cli-${target_label}"
  installed_binary="${version_dir}/yggterm"
  installed_headless_binary="${version_dir}/yggterm-headless"
  installed_mock_cli="${version_dir}/yggterm-mock-cli"
  ;;
esac

cp "${TMP_DIR}/${binary_name}" "${installed_binary}"
cp "${TMP_DIR}/${headless_binary_name}" "${installed_headless_binary}"
cp "${TMP_DIR}/${mock_cli_binary_name}" "${installed_mock_cli}"
chmod 0755 "${installed_binary}" || true
chmod 0755 "${installed_headless_binary}" || true
chmod 0755 "${installed_mock_cli}" || true

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

write_launcher_wrapper
write_mock_cli_wrapper

log "refreshing desktop integration"
"${installed_binary}" install integrate >/dev/null 2>&1 || true

log "installed yggterm ${release_version}"
log "binary: ${installed_binary}"
log "rerun this same install command any time to update manually"
bin_dir="${HOME}/.local/bin"
case ":${PATH:-}:" in
  *":${bin_dir}:"*) ;;
  *)
  log "add ${bin_dir} to PATH if you want the yggterm command in your shell"
  ;;
esac
