#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

[[ $(uname -s) == Linux ]] || { echo "Run this script on Debian or Ubuntu" >&2; exit 1; }
version=$(cargo pkgid --locked | sed 's/.*[#@]//')
if [[ ${GITHUB_REF_TYPE:-} == tag && ${GITHUB_REF_NAME:-} != "v$version" ]]; then
  echo "Release tag does not match package version $version" >&2
  exit 1
fi
arch=$(dpkg --print-architecture)
cargo build --locked --release --target-dir target

staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT
install -Dm755 target/release/marktype "$staging/usr/bin/marktype"
install -Dm644 packaging/linux/com.marktype.MarkType.desktop "$staging/usr/share/applications/com.marktype.MarkType.desktop"
install -Dm644 assets/icon.svg "$staging/usr/share/icons/hicolor/scalable/apps/com.marktype.MarkType.svg"
install -Dm644 LICENSE "$staging/usr/share/doc/marktype/copyright"
install -Dm644 NOTICE.md "$staging/usr/share/doc/marktype/NOTICE.md"
mkdir -p "$staging/DEBIAN" dist
cat > "$staging/DEBIAN/control" <<EOF
Package: marktype
Version: $version
Architecture: $arch
Section: editors
Priority: optional
Maintainer: MarkType Contributors <maintainers@marktype.invalid>
Depends: libgtk-3-0t64 | libgtk-3-0, libwebkit2gtk-4.1-0, xdg-utils
Homepage: https://github.com/microcrystallization/marktype
Description: Markdown and LaTeX editor
 MarkType provides a live preview, offline math rendering, and adaptive themes.
EOF
dpkg-deb --root-owner-group --build "$staging" "dist/MarkType-$version-linux-$arch.deb"
