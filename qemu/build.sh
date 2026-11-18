#!/usr/bin/env bash
# Build QEMU (aarch64-softmmu only) with the LabWired MMIO bridge patch.
#   qemu/build.sh            -> $QEMU_LW_ROOT/build/qemu-system-aarch64
# Env: QEMU_LW_ROOT (default /Volumes/LabWired/caches/qemu-labwired)
set -euo pipefail

QEMU_TAG="${QEMU_TAG:-v11.1.3}"
ROOT="${QEMU_LW_ROOT:-/Volumes/LabWired/caches/qemu-labwired}"
HERE="$(cd "$(dirname "$0")" && pwd)"
PATCH="$HERE/patches/0001-labwired-mmio-bridge.patch"
SRC="$ROOT/src"
BUILD="$ROOT/build"

if [[ "$(uname)" == Darwin ]]; then
    for f in ninja meson pkg-config glib pixman; do
        brew list --versions "$f" >/dev/null 2>&1 || brew install "$f"
    done
fi

mkdir -p "$ROOT"
if [[ ! -d "$SRC/.git" ]]; then
    git clone --depth 1 --branch "$QEMU_TAG" https://gitlab.com/qemu-project/qemu.git "$SRC"
fi

cd "$SRC"
if [[ "$(git describe --tags --exact-match 2>/dev/null)" != "$QEMU_TAG" ]]; then
    echo "error: $SRC is not at $QEMU_TAG" >&2
    exit 1
fi
# Always rebuild the tree from the pinned tag + the patch in this repo.
git reset -q --hard
git clean -fdq
git apply --index "$PATCH"

if [[ ! -f "$BUILD/build.ninja" ]]; then
    mkdir -p "$BUILD"
    (cd "$BUILD" && "$SRC/configure" --target-list=aarch64-softmmu \
        --disable-docs --disable-werror --disable-gtk --disable-sdl \
        --disable-cocoa --disable-vnc --disable-slirp)
fi
ninja -C "$BUILD"
"$BUILD/qemu-system-aarch64" -device help | grep -q labwired-mmio-bridge
echo "OK: $BUILD/qemu-system-aarch64 (QEMU $QEMU_TAG + labwired-mmio-bridge)"
