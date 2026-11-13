#!/usr/bin/env bash
# Build TailscaleKit (github.com/tailscale/libtailscale) into clients/apple/Vendor/ as one xcframework: iOS
# device, iOS simulator, macOS universal (arm64 + x86_64). Needs Xcode and Go (brew install go).
# Not committed; rerun after a fresh checkout or to update Tailscale. Without it the app builds
# without remote access over Tailscale (see scripts/xcodegen.sh).
#
# On a Mac: run it in the repository (builds into clients/apple/Vendor). Elsewhere: it runs on
# TSKIT_MAC (the environment's, else scripts/mac.env's, else MAC) over SSH, into ~/$MAC_DIR/clients/apple/Vendor there
# (MAC_DIR, default video-player).
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [ "$(uname)" = Darwin ]; then
  RUN=(env "DEST=$HERE/clients/apple" bash -s)
else
  # TSKIT_MAC and MAC_DIR given in the environment win over scripts/mac.env's (release-apple.sh
  # passes its own Mac and folder).
  ASKED=${TSKIT_MAC:-}; ASKED_DIR=${MAC_DIR:-}; . "$HERE/scripts/common.sh"
  TSKIT_MAC=${ASKED:-$TSKIT_MAC}; MAC_DIR=${ASKED_DIR:-$MAC_DIR}
  need "$TSKIT_MAC" "no build Mac: copy scripts/mac.env.example to scripts/mac.env and set MAC (or TSKIT_MAC), or run this on a Mac"
  RUN=(ssh "$TSKIT_MAC" "DEST=\$HOME/$MAC_DIR/clients/apple bash -s")
fi
"${RUN[@]}" <<'SCRIPT'
set -e; export PATH=/opt/homebrew/bin:/usr/local/bin:$PATH
mkdir -p ~/src && cd ~/src
[ -d libtailscale ] || git clone -q https://github.com/tailscale/libtailscale.git
cd libtailscale && git pull -q
# iOS device + simulator frameworks (upstream recipe)
(cd swift && make ios ios-sim >/tmp/tskit-ios.log 2>&1) || { tail -30 /tmp/tskit-ios.log; exit 1; }
# macOS: upstream builds host-arch only; build both slices and lipo them.
SDK=$(xcrun --sdk macosx --show-sdk-path)
for A in arm64:arm64 amd64:x86_64; do GA=${A%%:*}; CA=${A##*:}
  MACOSX_DEPLOYMENT_TARGET=14.0 CGO_ENABLED=1 GOOS=darwin GOARCH=$GA CC="clang -arch $CA -isysroot $SDK -mmacosx-version-min=14.0" \
    go build -buildmode=c-archive -o libtailscale_$CA.a >/tmp/tskit-mac-$CA.log 2>&1 || { tail -20 /tmp/tskit-mac-$CA.log; exit 1; }
done
lipo -create libtailscale_arm64.a libtailscale_x86_64.a -output libtailscale.a
cp libtailscale_arm64.h libtailscale.h 2>/dev/null || true
(cd swift && xcodebuild build -scheme "TailscaleKit (macOS)" -derivedDataPath build -configuration Release \
   -destination "generic/platform=macOS" ARCHS="arm64 x86_64" ONLY_ACTIVE_ARCH=NO CODE_SIGNING_ALLOWED=NO >/tmp/tskit-macfw.log 2>&1) \
   || { grep -E "error" /tmp/tskit-macfw.log | head -20; exit 1; }
P=swift/build/Build/Products
mkdir -p $DEST/Vendor && rm -rf $DEST/Vendor/TailscaleKit.xcframework
xcodebuild -create-xcframework \
  -framework $P/Release-iphoneos/TailscaleKit.framework \
  -framework $P/Release-iphonesimulator/TailscaleKit.framework \
  -framework $P/Release/TailscaleKit.framework \
  -output $DEST/Vendor/TailscaleKit.xcframework >/dev/null
lipo -info $P/Release/TailscaleKit.framework/Versions/A/TailscaleKit
du -sh $DEST/Vendor/TailscaleKit.xcframework
SCRIPT
