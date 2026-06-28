#!/usr/bin/env bash
# Build spice-transport for Android targets and generate Kotlin bindings.
#
# Prerequisites:
#   cargo install cargo-ndk
#   rustup target add aarch64-linux-android x86_64-linux-android
#   ANDROID_NDK_HOME set (or detected from ~/Android/Sdk/ndk/*)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
RUST_DIR="$SCRIPT_DIR/../rust"
OUT_DIR="$SCRIPT_DIR/../jniLibs"
KOTLIN_DIR="$SCRIPT_DIR/../kotlin"

# Detect NDK if not set
if [ -z "${ANDROID_NDK_HOME:-}" ]; then
    NDK_DIR="$(ls -d "$HOME/Android/Sdk/ndk"/*/ 2>/dev/null | sort -V | tail -1)"
    if [ -n "$NDK_DIR" ]; then
        export ANDROID_NDK_HOME="$NDK_DIR"
        echo "Using NDK: $ANDROID_NDK_HOME"
    else
        echo "ERROR: ANDROID_NDK_HOME not set and no NDK found in ~/Android/Sdk/ndk/" >&2
        exit 1
    fi
fi

cd "$RUST_DIR"

echo "==> Building for arm64-v8a..."
cargo ndk -t arm64-v8a -o "$OUT_DIR" build --release

echo "==> Building for x86_64..."
cargo ndk -t x86_64 -o "$OUT_DIR" build --release

echo "==> Generating Kotlin bindings..."
mkdir -p "$KOTLIN_DIR"

# Host (debug) build for binding generation — debug so metadata isn't stripped
echo "==> Building host library (debug) for uniffi-bindgen..."
cargo build

LIBPATH=""
if [ -f "target/debug/libspice_transport.so" ]; then
    LIBPATH="target/debug/libspice_transport.so"
elif [ -f "target/debug/libspice_transport.dylib" ]; then
    LIBPATH="target/debug/libspice_transport.dylib"
fi

if [ -z "$LIBPATH" ]; then
    echo "ERROR: Host build succeeded but library not found in target/debug/" >&2
    exit 1
fi

cargo run --bin uniffi-bindgen -- generate --library "$LIBPATH" \
    --language kotlin --out-dir "$KOTLIN_DIR" \
    --config uniffi.toml

# Verify generated bindings exist
if [ ! -f "$KOTLIN_DIR/sh/haven/spice/spice_transport.kt" ]; then
    echo "ERROR: uniffi-bindgen did not produce spice_transport.kt" >&2
    exit 1
fi

echo "==> Kotlin bindings generated in $KOTLIN_DIR"

echo "==> Done. Libraries in $OUT_DIR:"
find "$OUT_DIR" -name "*.so" -type f
