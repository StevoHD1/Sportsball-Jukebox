#!/bin/bash
# Builds the Rust engine as a static library for the Mac app.
#
# Run by the Xcode project's pre-build phase, and by CI. When run from
# Xcode, builds only the architectures Xcode is building ($ARCHS);
# otherwise builds a universal (arm64 + x86_64) library.
#
# Output: target/apple/libsbj_ffi.a
set -euo pipefail

cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
# Must match the app's deployment target (cpal's Core Audio backend needs 14.2).
export MACOSX_DEPLOYMENT_TARGET=14.2

if ! command -v cargo >/dev/null; then
    echo "error: cargo not found. Install Rust from https://rustup.rs" >&2
    exit 1
fi

archs="${ARCHS:-arm64 x86_64}"
libs=()
for arch in $archs; do
    case "$arch" in
        arm64) target=aarch64-apple-darwin ;;
        x86_64) target=x86_64-apple-darwin ;;
        *) echo "error: unsupported architecture $arch" >&2; exit 1 ;;
    esac
    # Always optimised: the audio engine must meet real-time deadlines even
    # in Debug app builds.
    cargo build --release -p sbj-ffi --target "$target"
    libs+=("target/$target/release/libsbj_ffi.a")
done

mkdir -p target/apple
lipo -create "${libs[@]}" -output target/apple/libsbj_ffi.a.tmp
mv target/apple/libsbj_ffi.a.tmp target/apple/libsbj_ffi.a
echo "built target/apple/libsbj_ffi.a ($archs)"
