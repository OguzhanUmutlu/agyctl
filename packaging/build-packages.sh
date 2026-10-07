#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:-0.1.0}"
ARCH="${2:-amd64}"
DIST_DIR="$(pwd)/dist"
BUILD_DIR="$(pwd)/build_pkg"

rm -rf "${DIST_DIR}" "${BUILD_DIR}"
mkdir -p "${DIST_DIR}" "${BUILD_DIR}"

cargo build --release

BIN_SRC="$(pwd)/target/release/agyctl"
strip "${BIN_SRC}"

tar -czf "${DIST_DIR}/agyctl-${VERSION}-x86_64-unknown-linux-gnu.tar.gz" -C target/release agyctl

DEB_ROOT="${BUILD_DIR}/deb"
mkdir -p "${DEB_ROOT}/DEBIAN"
mkdir -p "${DEB_ROOT}/usr/bin"
mkdir -p "${DEB_ROOT}/usr/lib/systemd/user"

cp "${BIN_SRC}" "${DEB_ROOT}/usr/bin/agyctl"
chmod 755 "${DEB_ROOT}/usr/bin/agyctl"

cat > "${DEB_ROOT}/usr/lib/systemd/user/agyctl.service" << EOF
[Unit]
Description=Antigravity Quota Cache Daemon (agyctl)
After=network.target

[Service]
Type=simple
ExecStart=/usr/bin/agyctl daemon --interval 10
Restart=always
RestartSec=10

[Install]
WantedBy=default.target
EOF
chmod 644 "${DEB_ROOT}/usr/lib/systemd/user/agyctl.service"

cat > "${DEB_ROOT}/DEBIAN/control" << EOF
Package: agyctl
Version: ${VERSION}
Section: utils
Priority: optional
Architecture: ${ARCH}
Maintainer: Antigravity Team
Description: Antigravity Control, Sync and Account Management CLI
 Unified CLI and background user service for Antigravity.
EOF

cat > "${DEB_ROOT}/DEBIAN/postinst" << 'EOF'
#!/bin/sh
set -e
if [ "$1" = "configure" ]; then
    systemctl --global daemon-reload 2>/dev/null || true
    systemctl --global enable agyctl.service 2>/dev/null || true
fi
EOF
chmod 755 "${DEB_ROOT}/DEBIAN/postinst"

cat > "${DEB_ROOT}/DEBIAN/prerm" << 'EOF'
#!/bin/sh
set -e
if [ "$1" = "remove" ]; then
    systemctl --global disable agyctl.service 2>/dev/null || true
fi
EOF
chmod 755 "${DEB_ROOT}/DEBIAN/prerm"

dpkg-deb --build "${DEB_ROOT}" "${DIST_DIR}/agyctl_${VERSION}_${ARCH}.deb"

APP_DIR="${BUILD_DIR}/AppDir"
mkdir -p "${APP_DIR}/usr/bin"
cp "${BIN_SRC}" "${APP_DIR}/usr/bin/agyctl"

cat > "${APP_DIR}/agyctl.desktop" << EOF
[Desktop Entry]
Name=agyctl
Exec=agyctl
Icon=agyctl
Type=Application
Categories=Utility;Development;
Terminal=true
EOF

cat > "${APP_DIR}/agyctl.svg" << 'EOF'
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" width="64" height="64">
  <rect width="64" height="64" rx="12" fill="#1e1e2e"/>
  <circle cx="32" cy="32" r="20" stroke="#89b4fa" stroke-width="4" fill="none"/>
  <polygon points="32,18 42,38 22,38" fill="#a6e3a1"/>
</svg>
EOF

cat > "${APP_DIR}/AppRun" << 'EOF'
#!/bin/sh
HERE="$(dirname "$(readlink -f "${0}")")"
export PATH="${HERE}/usr/bin:${PATH}"
exec "${HERE}/usr/bin/agyctl" "$@"
EOF
chmod 755 "${APP_DIR}/AppRun"

if command -v appimagetool >/dev/null 2>&1; then
    appimagetool "${APP_DIR}" "${DIST_DIR}/agyctl-${VERSION}-x86_64.AppImage"
else
    tar -czf "${DIST_DIR}/agyctl-${VERSION}-AppDir.tar.gz" -C "${BUILD_DIR}" AppDir
fi

cd "${DIST_DIR}"
sha256sum * > SHA256SUMS

rm -rf "${BUILD_DIR}"
echo "Generated release artifacts in ${DIST_DIR}"
