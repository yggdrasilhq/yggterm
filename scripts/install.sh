#!/usr/bin/env sh
# Keep this script POSIX-sh compatible because the documented install flow is:
# curl -fsSL .../install.sh | sh
set -eu
(set -o pipefail) >/dev/null 2>&1 && set -o pipefail

REPO="${YGGTERM_REPO:-yggdrasilhq/yggterm}"
LATEST_URL="https://github.com/${REPO}/releases/latest"
INSTALL_SCRATCH_ROOT="${YGGTERM_INSTALL_SCRATCH:-${HOME}/.yggterm/scratchpad/ynpm}"
mkdir -p "${INSTALL_SCRATCH_ROOT}"
TMP_DIR="$(mktemp -d "${INSTALL_SCRATCH_ROOT}/install-XXXXXX")"

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
# yggterm-direct-launcher-v4
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
use_headless=0
if [ "\${1:-}" = "server" ]; then
  use_headless=1
  if [ "\${2:-}" = "app" ] && [ "\${3:-}" = "launch" ]; then
    use_headless=0
  fi
fi
if [ "\${1:-}" = "--version" ] || [ "\${1:-}" = "-V" ] || [ "\${1:-}" = "version" ]; then
  use_headless=1
fi
if [ "\$use_headless" = "1" ]; then
  target_dir="\$(dirname "\$target")"
  if [ -x "\$target_dir/yggterm-headless" ]; then
    target="\$target_dir/yggterm-headless"
  fi
fi
if [ "${YGGTERM_ENABLE_ACCESSIBILITY:-0}" != "1" ] && [ -z "${NO_AT_BRIDGE+x}" ]; then
  export NO_AT_BRIDGE=1
fi
export YGGTERM_DIRECT_INSTALL_ROOT='${install_root}'
exec "\$target" "\$@"
EOF
  chmod 0755 "${launcher_path}" || true
}

write_headless_wrapper() {
  bin_dir="${HOME}/.local/bin"
  launcher_path="${bin_dir}/yggterm-headless"
  mkdir -p "${bin_dir}"
  cat > "${launcher_path}" <<EOF
#!/usr/bin/env sh
# yggterm-direct-launcher-v4
set -eu
ROOT='${install_root}'
target=""
latest_dir="\$(ls -td "\$ROOT"/versions/* 2>/dev/null | head -n 1 || true)"
if [ -n "\$latest_dir" ] && [ -x "\$latest_dir/yggterm-headless" ]; then
  target="\$latest_dir/yggterm-headless"
fi
if [ -z "\$target" ] || [ ! -x "\$target" ]; then
  target='${installed_headless_binary}'
fi
[ -x "\$target" ] || {
  printf '%s\n' 'yggterm-headless launcher: no runnable executable found' >&2
  exit 1
}
if [ "${YGGTERM_ENABLE_ACCESSIBILITY:-0}" != "1" ] && [ -z "${NO_AT_BRIDGE+x}" ]; then
  export NO_AT_BRIDGE=1
fi
export YGGTERM_DIRECT_INSTALL_ROOT='${install_root}'
exec "\$target" "\$@"
EOF
  chmod 0755 "${launcher_path}" || true
}

publish_aux_link() {
  # ynpm and ynpx are the same executable; the basename is their argv0
  # contract, so keep both links pointed at the versioned release binary.
  name="$1"
  target="$2"
  bin_dir="${HOME}/.local/bin"
  launcher_path="${bin_dir}/${name}"
  staged="${launcher_path}.ynpm-publish.$$"
  mkdir -p "${bin_dir}"
  rm -f "${staged}"
  ln -s "${target}" "${staged}"
  mv -f "${staged}" "${launcher_path}"
}

# ------------------------------------------------------------- ygg4 seed path
# The GUI-free seed install is the DEFAULT 4.0 entry. The 3.x direct/GUI
# installer stays available EXPLICITLY (`install.sh legacy` or
# YGGTERM_LEGACY=1) for version compatibility and is never removed. The
# seed path never touches the direct/GUI install root, never starts a
# daemon, and delegates the entire staging/publication/rollback transaction
# to `ynpm seed-activate`; this script only fetches verified bytes and
# publishes the command links afterwards.
seed_requested() {
  # Seed by default; the 3.x channel only by explicit operator intent.
  ! { [ "${YGGTERM_LEGACY:-0}" = "1" ] || [ "${1:-}" = "legacy" ]; }
}

seed_link_is_ours() {
  # true when an existing bin link points into the seed tree
  link_target="$(readlink "$1" 2>/dev/null || true)"
  case "${link_target}" in
    */.yggterm/seed/*) return 0 ;;
    *) return 1 ;;
  esac
}

seed_publish_link() {
  name="$1"
  link_path="${HOME}/.local/bin/${name}"
  if [ -e "${link_path}" ] || [ -L "${link_path}" ]; then
    if ! seed_link_is_ours "${link_path}"; then
      if [ "${YGG_MIGRATE_NATIVE:-0}" = "1" ]; then
        log "migrating existing ${name} command to the ygg4 seed (YGG_MIGRATE_NATIVE=1)"
      else
        log "existing non-seed ${name} at ${link_path} left in place; set YGG_MIGRATE_NATIVE=1 to take it over"
        return 0
      fi
    fi
  fi
  publish_aux_link "${name}" "${seed_root}/current/bin/${name}"
}

seed_install() {
  seed_root="${YGGTERM_SEED_ROOT:-${HOME}/.yggterm/seed}"
  seed_dist="${YGGTERM_SEED_DIST:-}"   # local dist override: tests and air-gapped hosts
  scratch="${YGGTERM_INSTALL_SCRATCH:-${HOME}/.yggterm/scratchpad/ygg4-install}"
  mkdir -p "${scratch}" "${HOME}/.local/bin"
  seed_tmp="$(mktemp -d "${scratch}/seed-XXXXXX")"

  requested_version="${YGGTERM_VERSION:-}"
  if [ -z "${requested_version}" ]; then
    latest_effective_url="$(curl -fsSL -o /dev/null -w '%{url_effective}' "${LATEST_URL}")"
    requested_version="$(printf '%s\n' "${latest_effective_url}" | sed -n 's#.*/tag/\(v[^/?#]*\).*#\1#p' | tail -n1 | sed 's/^v//')"
  fi
  requested_version="$(printf '%s' "${requested_version}" | sed 's/^v//')"
  [ -n "${requested_version}" ] || fail "could not resolve the seed version to install"
  release_tag="v${requested_version}"

  archive_name="ygg-seed-${target_label}.tar.gz"
  manifest_name="release-manifest.json"
  if [ -n "${seed_dist}" ]; then
    archive_path="${seed_dist}/${archive_name}"
    manifest_path="${seed_dist}/${manifest_name}"
    [ -f "${archive_path}" ] || fail "local seed dist has no ${archive_name}"
  else
    base_url="https://github.com/${REPO}/releases/download/${release_tag}"
    archive_path="${seed_tmp}/${archive_name}"
    manifest_path="${seed_tmp}/${manifest_name}"
    log "downloading ygg4 seed ${requested_version} for ${target_label}"
    curl -fL "${base_url}/${archive_name}" -o "${archive_path}" || {
      fail "seed archive ${archive_name} is not published for ${release_tag}"
    }
  fi

  # Checksum verification is mandatory: an unverifiable archive fails the
  # install, it is never installed unverified.
  checksum_path="${seed_tmp}/${archive_name}.sha256"
  if [ -n "${seed_dist}" ]; then
    cp "${seed_dist}/${archive_name}.sha256" "${checksum_path}" || {
      fail "local seed dist has no ${archive_name}.sha256"
    }
  else
    curl -fsSL "${base_url}/${archive_name}.sha256" -o "${checksum_path}" || {
      fail "no checksum published for ${archive_name}; refusing an unverifiable download"
    }
  fi
  expected="$(awk '{print $1}' "${checksum_path}")"
  case "${expected}" in
    [0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]*) ;;
    *) fail "published checksum for ${archive_name} is not a sha256 digest" ;;
  esac
  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "${archive_path}" | awk '{print $1}')"
  else
    actual="$(shasum -a 256 "${archive_path}" | awk '{print $1}')"
  fi
  [ "${expected}" = "${actual}" ] || fail "seed checksum verification failed for ${archive_name}"

  # The archive name above is constructed locally, but the manifest is
  # release-published content: keep it untrusted until ynpm validates it.
  if [ -n "${seed_dist}" ]; then
    cp "${manifest_path}" "${seed_tmp}/${manifest_name}" 2>/dev/null || true
    manifest_path="${seed_tmp}/${manifest_name}"
    [ -f "${manifest_path}" ] || fail "local seed dist has no ${manifest_name}"
  else
    curl -fsSL "${base_url}/${manifest_name}" -o "${manifest_path}" || {
      fail "release manifest not published for ${release_tag}"
    }
  fi

  # Extract the bootstrap only (the transaction owner we are about to run);
  # seed-activate re-extracts and verifies the archive itself, staged.
  tar -xzf "${archive_path}" -C "${seed_tmp}" ynpm || {
    fail "seed archive did not carry an executable ynpm bootstrap"
  }
  [ -x "${seed_tmp}/ynpm" ] || fail "extracted ynpm bootstrap is not executable"

  log "activating seed generation (staged by ynpm seed-activate)"
  "${seed_tmp}/ynpm" seed-activate \
    --archive "${archive_path}" \
    --manifest "${manifest_path}" \
    --package ygg-seed \
    --expect-version "${requested_version}" \
    --json || fail "seed-activate refused the transaction"

  # Only now are commands published, symlinking through current/ so later
  # activations are picked up without touching these links again.
  seed_publish_link ygg
  seed_publish_link ynpm
  seed_publish_link ynpx

  active_generation="$(readlink "${seed_root}/current" 2>/dev/null || echo '?')"
  log "installed ygg4 seed ${requested_version} (${active_generation})"
  log "commands: ${HOME}/.local/bin/{ygg,ynpm,ynpx}"
  case ":${PATH:-}:" in
    *":${HOME}/.local/bin:"*) ;;
    *) log "add ${HOME}/.local/bin to PATH to use the ygg command" ;;
  esac
  log "this host can now be operated over ssh as a ygg system; that is a remote-operator capability, not local worker readiness"
  log "previous generations are preserved under ${seed_root}/generations"
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

if seed_requested "$1"; then
  seed_status=0
  seed_install || seed_status=$?
  [ -n "${seed_tmp:-}" ] && rm -rf "${seed_tmp}" 2>/dev/null
  exit "${seed_status}"
fi

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

# After bootstrap, ynpm owns all later release updates and keeps the complete
# rollback history. The installer remains the no-dependency bootstrap only.
if [ -n "${current_version}" ] && [ -x "${install_root}/versions/${current_version}/ynpm" ]; then
  YGGTERM_DIRECT_INSTALL_ROOT="${install_root}" \
    exec "${install_root}/versions/${current_version}/ynpm" self-update
fi

if [ -n "${current_version}" ] && [ "${current_version}" = "${release_version}" ]; then
  log "yggterm ${release_version} is already installed"
  current_binary="$(
    sed -n 's/.*"active_executable"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "${state_path}" | head -n1
  )"
  if [ -n "${current_binary}" ] && [ -x "${current_binary}" ]; then
    installed_binary="${current_binary}"
    installed_headless_binary="$(dirname "${current_binary}")/yggterm-headless"
    installed_ynpm="${install_root}/versions/${current_version}/ynpm"
    installed_ynpx="${install_root}/versions/${current_version}/ynpx"
    write_launcher_wrapper
    write_headless_wrapper
    if [ -x "${installed_ynpm}" ] && [ -x "${installed_ynpx}" ]; then
      publish_aux_link ynpm "${installed_ynpm}"
      publish_aux_link ynpx "${installed_ynpx}"
    fi
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
  installed_binary="${version_dir}/yggterm.exe"
  installed_headless_binary="${version_dir}/yggterm-headless.exe"
  ;;
  *)
    binary_name="yggterm-${target_label}"
    headless_binary_name="yggterm-headless-${target_label}"
    ynpm_binary_name="ynpm-${target_label}"
    ynpx_binary_name="ynpx-${target_label}"
    installed_binary="${version_dir}/yggterm"
    installed_headless_binary="${version_dir}/yggterm-headless"
    installed_ynpm="${version_dir}/ynpm"
    installed_ynpx="${version_dir}/ynpx"
    ;;
esac

cp "${TMP_DIR}/${binary_name}" "${installed_binary}"
cp "${TMP_DIR}/${headless_binary_name}" "${installed_headless_binary}"
cp "${TMP_DIR}/${ynpm_binary_name}" "${installed_ynpm}"
cp "${TMP_DIR}/${ynpx_binary_name}" "${installed_ynpx}"
chmod 0755 "${installed_binary}" || true
chmod 0755 "${installed_headless_binary}" || true
chmod 0755 "${installed_ynpm}" "${installed_ynpx}" || true

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
write_headless_wrapper
publish_aux_link ynpm "${installed_ynpm}"
publish_aux_link ynpx "${installed_ynpx}"

log "refreshing desktop integration"
"${installed_binary}" install integrate >/dev/null 2>&1 || true

log "installed yggterm ${release_version}"
log "binary: ${installed_binary}"
log "future updates are owned by ynpm; run 'ynpm self-update'"
bin_dir="${HOME}/.local/bin"
case ":${PATH:-}:" in
  *":${bin_dir}:"*) ;;
  *)
  log "add ${bin_dir} to PATH if you want the yggterm command in your shell"
  ;;
esac
