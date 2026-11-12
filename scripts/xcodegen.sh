#!/usr/bin/env bash
# Generate clients/apple/Moonrise.xcodeproj (run on the Mac, in the repository): with TailscaleKit
# when clients/apple/Vendor/TailscaleKit.xcframework exists (scripts/build-tailscalekit.sh), else
# without remote access over Tailscale (the app builds and runs; Settings has no Tailscale section).
#
# First checks the Mac's Xcode and XcodeGen against clients/apple/Config/toolchain.env: another
# version prints a warning and carries on; no XcodeGen, or an Xcode older than the recorded major,
# stops here.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../clients/apple"
. Config/toolchain.env

xcode=$(xcodebuild -version 2>/dev/null | sed -n 's/^Xcode \([0-9.]*\).*/\1/p' | head -1 || true)
if [ -z "$xcode" ]; then
  echo "warning: couldn't read the Xcode version (xcodebuild -version); the apps are built with Xcode $XCODE_VERSION" >&2
elif [ "${xcode%%.*}" -lt "${XCODE_VERSION%%.*}" ]; then
  echo "error: Xcode $xcode is too old: the apps need Xcode ${XCODE_VERSION%%.*} or later (built with $XCODE_VERSION, clients/apple/Config/toolchain.env)" >&2
  exit 1
elif [ "$xcode" != "$XCODE_VERSION" ]; then
  echo "warning: Xcode $xcode here; the apps are built and tested with Xcode $XCODE_VERSION (clients/apple/Config/toolchain.env)" >&2
fi
if ! command -v xcodegen >/dev/null; then
  echo "error: xcodegen not found: install XcodeGen $XCODEGEN_VERSION (brew install xcodegen, or mint install yonaskolb/XcodeGen@$XCODEGEN_VERSION)" >&2
  exit 1
fi
xcodegen_have=$(xcodegen --version 2>/dev/null | sed -n 's/^Version: *//p' | head -1 || true)
if [ "$xcodegen_have" != "$XCODEGEN_VERSION" ]; then
  echo "warning: XcodeGen ${xcodegen_have:-of unknown version} here; the project is generated with XcodeGen $XCODEGEN_VERSION (clients/apple/Config/toolchain.env)" >&2
fi

# Before the Apple project moved to clients/apple/, TailscaleKit was built into Vendor/ at the
# repository root; it isn't tracked, so git doesn't move it: stop with the command that does,
# rather than build without remote access.
if [ -d ../../Vendor/TailscaleKit.xcframework ] && [ ! -d Vendor/TailscaleKit.xcframework ]; then
  echo "error: TailscaleKit is in its old place, Vendor/ at the repository root: the Apple project moved to clients/apple/, so move it: mv '$(cd ../.. && pwd)/Vendor' '$PWD/Vendor'" >&2
  exit 1
fi
if [ -d Vendor/TailscaleKit.xcframework ]; then
  export MOONRISE_TAILSCALE=YES
else
  export MOONRISE_TAILSCALE=NO
  echo "note: clients/apple/Vendor/TailscaleKit.xcframework not found: building without Tailscale (scripts/build-tailscalekit.sh makes it)" >&2
fi
exec xcodegen generate "$@"
