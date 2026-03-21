#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
DIST_DIR="${ROOT_DIR}/dist"
BIN_PATH="${ROOT_DIR}/target/release/yggterm"
DEB_REVISION="${DEB_REVISION:-1}"
ARCH="$(dpkg-architecture -qDEB_HOST_ARCH)"
RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-1.94.0}"
CARGO_CMD=(cargo "+${RUSTUP_TOOLCHAIN}")
VERSION="$("${CARGO_CMD[@]}" pkgid -p yggterm | sed 's/.*#//')"
DEB_VERSION="${VERSION}-${DEB_REVISION}"
PKG_NAME="yggterm"
STAGE_DIR="${ROOT_DIR}/.yggterm-state/deb/${PKG_NAME}_${DEB_VERSION}_${ARCH}"

mkdir -p "$DIST_DIR"

pushd "$ROOT_DIR" >/dev/null
"${CARGO_CMD[@]}" build --release -p yggterm --no-default-features
popd >/dev/null

rm -rf "$STAGE_DIR"
mkdir -p \
  "$STAGE_DIR/DEBIAN" \
  "$STAGE_DIR/usr/bin" \
  "$STAGE_DIR/usr/lib/yggterm" \
  "$STAGE_DIR/usr/share/doc/${PKG_NAME}"

install -m 0755 "$BIN_PATH" "$STAGE_DIR/usr/lib/yggterm/yggterm-bin"
cat > "$STAGE_DIR/usr/bin/yggterm" <<'WRAPPER'
#!/usr/bin/env bash
set -euo pipefail
exec /usr/lib/yggterm/yggterm-bin "$@"
WRAPPER
chmod 0755 "$STAGE_DIR/usr/bin/yggterm"
install -m 0644 "$ROOT_DIR/debian/copyright" "$STAGE_DIR/usr/share/doc/${PKG_NAME}/copyright"
gzip -c "$ROOT_DIR/debian/changelog" > "$STAGE_DIR/usr/share/doc/${PKG_NAME}/changelog.Debian.gz"

SHLIBS_LINE="$(cd "$ROOT_DIR" && dpkg-shlibdeps -O -e "$STAGE_DIR/usr/lib/yggterm/yggterm-bin" | sed -n 's/^shlibs:Depends=//p')"
if [[ -z "$SHLIBS_LINE" ]]; then
  SHLIBS_LINE="libc6"
fi
GUI_DEPS="libx11-6 | libwayland-client0, libxkbcommon0, libgl1"
DEPENDS_LINE="${SHLIBS_LINE}, ${GUI_DEPS}"

INSTALLED_SIZE="$(du -sk "$STAGE_DIR" | awk '{print $1}')"

cat > "$STAGE_DIR/DEBIAN/control" <<CONTROL
Package: ${PKG_NAME}
Version: ${DEB_VERSION}
Section: utils
Priority: optional
Architecture: ${ARCH}
Depends: ${DEPENDS_LINE}
Maintainer: Yggdrasil HQ <avi@gour.top>
Homepage: https://github.com/yggdrasilhq/yggterm
Installed-Size: ${INSTALLED_SIZE}
Description: Yggdrasil Terminal
 Rust-first terminal workspace with a Dioxus desktop shell,
 server-owned PTYs, and an embedded xterm.js terminal surface.
CONTROL

OUT_DEB="${DIST_DIR}/${PKG_NAME}_${DEB_VERSION}_${ARCH}.deb"
fakeroot dpkg-deb --build "$STAGE_DIR" "$OUT_DEB" >/dev/null
sha256sum "$OUT_DEB" > "${OUT_DEB}.sha256"

echo "Deb package: $OUT_DEB"
echo "Deb checksum: ${OUT_DEB}.sha256"
