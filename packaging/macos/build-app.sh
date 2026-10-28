#!/usr/bin/env bash
# Build OpenHE.app (macOS) from a release build of openhe-app.
#
# Usage: packaging/macos/build-app.sh [--no-sign] [--skip-build] [--zip]
#
# Output: dist/macos/OpenHE.app (and dist/macos/OpenHE-<version>-macos.zip with --zip)
# Env:    OPENHE_VERSION   override the bundle version (default: version of crates/openhe-app)
#         CARGO_TARGET     build for a specific target triple, e.g. aarch64-apple-darwin
set -euo pipefail

SIGN=1
BUILD=1
ZIP=0
for arg in "$@"; do
    case "$arg" in
        --no-sign) SIGN=0 ;;
        --skip-build) BUILD=0 ;;
        --zip) ZIP=1 ;;
        -h | --help)
            sed -n '2,8p' "$0"
            exit 0
            ;;
        *)
            echo "unknown argument: $arg" >&2
            exit 2
            ;;
    esac
done

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

# Bundle version: env override, else the first `version = "..."` in the app crate's manifest.
VERSION="${OPENHE_VERSION:-}"
if [ -z "$VERSION" ] && [ -f crates/openhe-app/Cargo.toml ]; then
    VERSION="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' crates/openhe-app/Cargo.toml | head -n 1)"
fi
VERSION="${VERSION:-0.1.0}"

if [ "$BUILD" -eq 1 ]; then
    if [ -n "${CARGO_TARGET:-}" ]; then
        cargo build --locked --release -p openhe-app --target "$CARGO_TARGET"
    else
        cargo build --locked --release -p openhe-app
    fi
fi

if [ -n "${CARGO_TARGET:-}" ]; then
    BIN="target/$CARGO_TARGET/release/openhe-app"
else
    BIN="target/release/openhe-app"
fi
ICON="assets/icons/openhe.icns"
for f in "$BIN" "$ICON"; do
    if [ ! -f "$f" ]; then
        echo "missing $f" >&2
        exit 1
    fi
done

OUT="dist/macos"
APP="$OUT/OpenHE.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp "$BIN" "$APP/Contents/MacOS/openhe-app"
chmod 755 "$APP/Contents/MacOS/openhe-app"
cp "$ICON" "$APP/Contents/Resources/openhe.icns"

cat > "$APP/Contents/Info.plist" << PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleExecutable</key>
    <string>openhe-app</string>
    <key>CFBundleIconFile</key>
    <string>openhe</string>
    <key>CFBundleIdentifier</key>
    <string>dev.openhe.OpenHE</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>OpenHE</string>
    <key>CFBundleDisplayName</key>
    <string>OpenHE</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>CFBundleVersion</key>
    <string>${VERSION}</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.utilities</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
PLIST

if command -v plutil > /dev/null 2>&1; then
    plutil -lint "$APP/Contents/Info.plist" > /dev/null
fi

if [ "$SIGN" -eq 1 ] && command -v codesign > /dev/null 2>&1; then
    # Ad-hoc signature: enough to run locally. Gatekeeper still asks on first launch for downloaded copies.
    codesign -s - --force --deep "$APP"
fi

echo "Built $APP (version $VERSION)"

if [ "$ZIP" -eq 1 ]; then
    ZIPFILE="OpenHE-${VERSION}-macos.zip"
    rm -f "${OUT:?}/$ZIPFILE"
    # ditto keeps the bundle's permissions and symlinks; plain zip can break them.
    (cd "$OUT" && ditto -c -k --sequesterRsrc --keepParent OpenHE.app "$ZIPFILE")
    echo "Wrote $OUT/$ZIPFILE"
fi
