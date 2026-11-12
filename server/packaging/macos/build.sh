#!/bin/bash
# Builds the macOS server for Apple silicon and Intel and joins them into one universal binary,
# then packs it with the install scripts:
#
#   server/packaging/macos/build.sh            # → server/target/macos/moonrise-server-<version>-macos.tar.gz
#
# Needs a Mac with Xcode's command line tools and rustup: the toolchain is the one pinned in
# server/rust-toolchain.toml, and the two macOS targets are added to it here. The built-in
# TMDB/OpenSubtitles keys are compiled in when MOONRISE_BUILTIN_TMDB_KEY /
# MOONRISE_BUILTIN_OPENSUBTITLES_KEY are set in the environment.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
# Runs on macOS 12 and later.
export MACOSX_DEPLOYMENT_TARGET=${MACOSX_DEPLOYMENT_TARGET:-12.0}
if command -v rustup >/dev/null; then rustup target add aarch64-apple-darwin x86_64-apple-darwin; fi
for t in aarch64-apple-darwin x86_64-apple-darwin; do
  cargo build --release --locked --target "$t"
done
OUT=target/macos/moonrise-server-$VERSION-macos
# Only this version in there: CI restores target/ from a cache, and looks for one package.
rm -rf target/macos && mkdir -p "$OUT"
lipo -create -output "$OUT/moonrise-server" target/aarch64-apple-darwin/release/moonrise-server target/x86_64-apple-darwin/release/moonrise-server
# Ad-hoc signature (Apple silicon runs nothing unsigned).
codesign --force --sign - "$OUT/moonrise-server"
cp packaging/macos/install.sh packaging/macos/uninstall.sh "$OUT/"
cp LICENSE "$OUT/"
tar -C target/macos -czf "$OUT.tar.gz" "$(basename "$OUT")"
lipo -info "$OUT/moonrise-server"
echo "$OUT.tar.gz"
