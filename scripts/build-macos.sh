#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

[[ $(uname -s) == Darwin ]] || { echo "Run this script on macOS" >&2; exit 1; }
version=$(cargo pkgid --locked | sed 's/.*[#@]//')
if [[ ${GITHUB_REF_TYPE:-} == tag && ${GITHUB_REF_NAME:-} != "v$version" ]]; then
  echo "Release tag does not match package version $version" >&2
  exit 1
fi
case $(uname -m) in
  arm64) arch=arm64 ;;
  x86_64) arch=x64 ;;
  *) echo "Unsupported macOS architecture" >&2; exit 1 ;;
esac

export MACOSX_DEPLOYMENT_TARGET=13.3
cargo build --locked --release --target-dir target
staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT
app="$staging/MarkType.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" dist
install -m 755 target/release/marktype "$app/Contents/MacOS/marktype"
install -m 644 packaging/macos/Info.plist "$app/Contents/Info.plist"
install -m 644 packaging/macos/MarkType.icns LICENSE NOTICE.md "$app/Contents/Resources/"
plutil -insert CFBundleShortVersionString -string "$version" "$app/Contents/Info.plist"
plutil -insert CFBundleVersion -string "${version%%-*}" "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist"
codesign --force --sign - "$app"
codesign --verify --strict "$app"
ln -s /Applications "$staging/Applications"
hdiutil create -ov -format UDZO -volname MarkType -srcfolder "$staging" "dist/MarkType-$version-macos-$arch.dmg"
