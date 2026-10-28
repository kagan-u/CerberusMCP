#!/usr/bin/env bash
# Stage CerberusMCP.app + a .dmg from a CMake build directory.
#   usage: package-macos.sh <build-dir> <version>   (version like 0.4.1, no leading v)
set -euo pipefail

BUILD="${1:?usage: package-macos.sh <build-dir> <version>}"
VER="${2:?usage: package-macos.sh <build-dir> <version>}"
SRC="$(cd "$(dirname "$0")/.." && pwd)"

APP="CerberusMCP.app"
rm -rf "$APP" "CerberusMCP_${VER}_macos.dmg"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp "$BUILD/CerberusMCP" "$APP/Contents/MacOS/CerberusMCP"
cp -R "$SRC/assets" "$APP/Contents/Resources/assets"
cp "$SRC/assets/logo.png" "$APP/Contents/Resources/logo.png"

sed "s/@CERBERUS_GUI_VERSION@/$VER/g" "$SRC/packaging/macos/Info.plist.in" \
    > "$APP/Contents/Info.plist"

# Ad-hoc sign so Gatekeeper quarantine is the only remaining friction.
codesign --force --deep -s - "$APP" 2>/dev/null || true

hdiutil create -volname "CerberusMCP" -srcfolder "$APP" -ov -format UDZO \
    "CerberusMCP_${VER}_macos.dmg"
tar -czf "CerberusMCP_${VER}_macos.app.tar.gz" "$APP"
ls -la "CerberusMCP_${VER}_macos.dmg" "CerberusMCP_${VER}_macos.app.tar.gz"
