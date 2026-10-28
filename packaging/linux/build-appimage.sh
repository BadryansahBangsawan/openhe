#!/usr/bin/env bash
# Build an OpenHE AppImage with linuxdeploy.
#
# Usage: packaging/linux/build-appimage.sh [--skip-build]
#
# Output: dist/linux/OpenHE-<version>-x86_64.AppImage
# Env:    OPENHE_VERSION   version string (default: version of crates/openhe-app)
#         LINUXDEPLOY      path to a linuxdeploy binary (default: `linuxdeploy` on PATH,
#                          else downloaded to dist/tools/ from the linuxdeploy GitHub releases)
#         LINUXDEPLOY_VERSION  linuxdeploy release tag to download (default: continuous, see TODO below)
#
# udev: an AppImage cannot install files outside itself, so the udev rule is NOT applied by this script.
# Users install packaging/linux/70-openhe.rules once (see packaging/linux/README.md). The rule is also
# copied into the AppImage under usr/share/openhe/ so it can be extracted with
# `./OpenHE-*.AppImage --appimage-extract usr/share/openhe/70-openhe.rules`.
set -euo pipefail

BUILD=1
for arg in "$@"; do
    case "$arg" in
        --skip-build) BUILD=0 ;;
        -h | --help)
            sed -n '2,15p' "$0"
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

VERSION="${OPENHE_VERSION:-}"
if [ -z "$VERSION" ] && [ -f crates/openhe-app/Cargo.toml ]; then
    VERSION="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' crates/openhe-app/Cargo.toml | head -n 1)"
fi
VERSION="${VERSION:-0.1.0}"

if [ "$BUILD" -eq 1 ]; then
    cargo build --locked --release -p openhe-app
fi

BIN="target/release/openhe-app"
if [ ! -f "$BIN" ]; then
    echo "missing $BIN" >&2
    exit 1
fi

OUT="dist/linux"
APPDIR="$OUT/AppDir"
rm -rf "$APPDIR"
mkdir -p "$OUT" "$APPDIR/usr/share/openhe"
cp packaging/linux/70-openhe.rules "$APPDIR/usr/share/openhe/70-openhe.rules"

# linuxdeploy: PATH, $LINUXDEPLOY, or download.
# TODO: pin LINUXDEPLOY_VERSION to a dated release tag from
# https://github.com/linuxdeploy/linuxdeploy/releases (not looked up offline, so `continuous` stays for now)
# and verify the downloaded file's SHA-256 against a value recorded here before running it.
LINUXDEPLOY_VERSION="${LINUXDEPLOY_VERSION:-continuous}"
LD="${LINUXDEPLOY:-}"
if [ -z "$LD" ]; then
    if command -v linuxdeploy > /dev/null 2>&1; then
        LD="linuxdeploy"
    else
        mkdir -p dist/tools
        LD="dist/tools/linuxdeploy-x86_64.AppImage"
        if [ ! -x "$LD" ]; then
            curl -fsSL -o "$LD" \
                "https://github.com/linuxdeploy/linuxdeploy/releases/download/${LINUXDEPLOY_VERSION}/linuxdeploy-x86_64.AppImage"
            chmod +x "$LD"
        fi
    fi
fi

# Icons: linuxdeploy names the installed icon after the file, so every size is copied as openhe.png.
ICONARGS=()
ICONTMP="$(mktemp -d)"
trap 'rm -rf "$ICONTMP"' EXIT
for size in 16 32 48 64 128 256 512; do
    mkdir -p "$ICONTMP/$size"
    cp "assets/icons/png/openhe-$size.png" "$ICONTMP/$size/openhe.png"
    ICONARGS+=(--icon-file "$ICONTMP/$size/openhe.png")
done

# FUSE is often missing on CI; the AppImage tools can unpack themselves instead.
export APPIMAGE_EXTRACT_AND_RUN=1
export LINUXDEPLOY_OUTPUT_VERSION="$VERSION"
export OUTPUT="$OUT/OpenHE-${VERSION}-x86_64.AppImage"

"$LD" \
    --appdir "$APPDIR" \
    --executable "$BIN" \
    --desktop-file packaging/linux/openhe.desktop \
    "${ICONARGS[@]}" \
    --output appimage

# linuxdeploy writes the AppImage into the current directory unless OUTPUT is honoured by the plugin.
for f in OpenHE*.AppImage; do
    if [ -f "$f" ]; then
        mv -f "$f" "$OUT/"
    fi
done

echo "Built AppImage in $OUT/"
ls -1 "$OUT"/*.AppImage
