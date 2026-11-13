#!/usr/bin/env bash
# App Store screenshots of the built-in demo (the Blender Foundation's open movies, no server) on
# simulators of the sizes App Store Connect asks for, into clients/apple/fastlane/screenshots
# (scripts/release-apple.sh screenshots uploads them):
#
#   scripts/store-shots.sh [phone|ipad|tv|all]   (default all)
#
#   phone  iPhone 17 Pro Max, 6.9" (1320 x 2868)       -> screenshots/ios/en-US/<n>-phone-<name>.jpg
#   ipad   iPad Pro 13-inch (M5), 13" (2064 x 2752)     -> screenshots/ios/en-US/<n>-ipad-<name>.jpg
#   tv     Apple TV 4K (3rd generation) (3840 x 2160)   -> screenshots/appleTV/en-US/<n>-<name>.jpg
#
# The Mac's (2880 x 1800) are taken by hand on a Mac running macOS 26: the app needs it.
# It runs on the release Mac, RELEASE_MAC, as scripts/release-apple.sh does (never MAC or FASTLANE_MAC,
# which may name someone's own Mac; a host in PERSONAL_MACS is refused; scripts/common.sh).
# It builds the Debug simulator app (its launch seams open the demo, a title, the player; release
# builds have none) in its own copy on the Mac, ~/$SHOTS_DIR (default moonrise-store-shots), and
# uses simulators of its own, "Moonrise Store <device>", made the first time (--delete removes them).
# The player shots stream the films from their public hosts, so the Mac needs the internet.
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/common.sh"
use_release_mac "${SHOTS_DIR:-moonrise-store-shots}"
release_mac_ok || exit 1
OUT="$MOONRISE_ROOT/$APPLE_DIR/fastlane/screenshots"
DELETE=; [ "${1:-}" = --delete ] && { DELETE=1; shift; }
WHICH=${1:-all}
case "$WHICH" in phone|ipad|tv) KINDS=$WHICH ;; all) KINDS="phone ipad tv" ;; *) echo "usage: $0 [--delete] [phone|ipad|tv|all]" >&2; exit 2 ;; esac

device_type() {
  case "$1" in
    phone) echo "com.apple.CoreSimulator.SimDeviceType.iPhone-17-Pro-Max" ;;
    ipad) echo "com.apple.CoreSimulator.SimDeviceType.iPad-Pro-13-inch-M5-12GB" ;;
    tv) echo "com.apple.CoreSimulator.SimDeviceType.Apple-TV-4K-3rd-generation-4K" ;;
  esac
}
# sim_udid <kind>: the simulator "Moonrise Store <kind>", made with the newest runtime when missing.
sim_udid() {
  local name="Moonrise Store $1" os=iOS
  [ "$1" = tv ] && os=tvOS
  ssh "$MAC" "u=\$(xcrun simctl list devices available | sed -n 's/^ *$name (\\([0-9A-F-]\\{36\\}\\)).*/\\1/p' | head -1)
[ -n \"\$u\" ] || u=\$(xcrun simctl create '$name' $(device_type "$1") \$(xcrun simctl list runtimes available | grep -o 'com.apple.CoreSimulator.SimRuntime.$os-[0-9-]*' | tail -1))
echo \$u"
}

if [ -n "$DELETE" ]; then
  for k in $KINDS; do
    ssh "$MAC" "xcrun simctl list devices | sed -n 's/^ *Moonrise Store $k (\\([0-9A-F-]\\{36\\}\\)).*/\\1/p' | while read -r u; do xcrun simctl shutdown \$u 2>/dev/null; xcrun simctl delete \$u && echo deleted \$u; done"
  done
  exit 0
fi

# The shots: <file name> <seconds to wait> <seams (MOONRISE_ names without the prefix)>. Tears of
# Steel's English subtitles are track index 2 (DemoClient).
SHOTS_TOUCH=(
  "1-home 20 DEMO=1"
  "2-title 15 DEMO=1 OPEN_ITEM=demo-tos"
  "3-player 40 DEMO=1 AUTOPLAY=demo-tos AUTOPLAY_AT=208 AUTOPLAY_SUB=t:2 AUTOPLAY_PAUSE=212 LANDSCAPE=1"
  "4-title 15 DEMO=1 OPEN_ITEM=demo-sintel"
)
SHOTS_TV=(
  "1-home 25 DEMO=1"
  "2-title 15 DEMO=1 OPEN_ITEM=demo-tos"
  "3-player 40 DEMO=1 AUTOPLAY=demo-tos AUTOPLAY_AT=208 AUTOPLAY_SUB=t:2 AUTOPLAY_PAUSE=212"
  "4-title 15 DEMO=1 OPEN_ITEM=demo-sintel"
)

# strip_jpeg <file>: drop the Exif / XMP (APP1) and comment segments sips writes, keeping the
# pixels (scripts/export-public.sh refuses images that carry text).
strip_jpeg() {
  python3 -I - "$1" <<'PY'
import struct, sys
p = sys.argv[1]
d = open(p, "rb").read()
out, i = bytearray(d[:2]), 2
while i + 4 <= len(d) and d[i] == 0xFF and d[i + 1] != 0xDA:
    n = 2 + struct.unpack(">H", d[i + 2:i + 4])[0]
    if d[i + 1] not in (0xE1, 0xFE):
        out += d[i:i + n]
    i += n
out += d[i:]
open(p, "wb").write(out)
PY
}

built_sim=; built_tv=
for k in $KINDS; do
  if [ "$k" = tv ]; then
    [ -n "$built_tv" ] || { "$MOONRISE_ROOT/scripts/build.sh" tvos; built_tv=1; }
    APP=build-tvos/Build/Products/Debug-appletvsimulator/Moonrise.app; DEST="$OUT/appleTV/en-US"; SHOTS=("${SHOTS_TV[@]}"); PREFIX=
  else
    [ -n "$built_sim" ] || { "$MOONRISE_ROOT/scripts/build.sh" sim; built_sim=1; }
    APP=build-sim/Build/Products/Debug-iphonesimulator/Moonrise.app; DEST="$OUT/ios/en-US"; SHOTS=("${SHOTS_TOUCH[@]}"); PREFIX=$k-
  fi
  U=$(sim_udid "$k" | tail -1)
  [ -n "$U" ] || { echo "error: no $k simulator" >&2; exit 1; }
  echo "==> $k: simulator $U"
  mkdir -p "$DEST"
  ssh "$MAC" "xcrun simctl bootstatus $U -b >/dev/null 2>&1 || xcrun simctl boot $U
$([ "$k" = tv ] || echo "xcrun simctl status_bar $U override --time 9:41 --batteryState charged --batteryLevel 100 --wifiBars 3 --cellularMode notSupported")
xcrun simctl install $U ~/$MAC_DIR/$APP"
  for s in "${SHOTS[@]}"; do
    read -r name wait seams <<<"$s"
    env=; land=
    # The iPhone plays in landscape (turned upright here); the iPad shows the player in portrait.
    for kv in $seams; do
      if [ "$kv" = LANDSCAPE=1 ]; then [ "$k" = phone ] || continue; land=1; fi
      env="$env SIMCTL_CHILD_MOONRISE_$kv"
    done
    file="${name%%-*}-$PREFIX${name#*-}.jpg"
    echo "    $file"
    ssh "$MAC" "xcrun simctl terminate $U $APP_ID >/dev/null 2>&1; sleep 1
env$env SIMCTL_CHILD_MOONRISE_NO_REPORT=1 xcrun simctl launch $U $APP_ID >/dev/null
sleep $wait
xcrun simctl io $U screenshot --type=png /tmp/moonrise-store-shot.png >/dev/null 2>&1
$([ -n "$land" ] && echo "sips -r 270 /tmp/moonrise-store-shot.png >/dev/null")
sips -s format jpeg -s formatOptions 90 /tmp/moonrise-store-shot.png --out /tmp/moonrise-store-shot.jpg >/dev/null
xcrun simctl terminate $U $APP_ID >/dev/null 2>&1 || true"
    scp -q "$MAC:/tmp/moonrise-store-shot.jpg" "$DEST/$file"
    strip_jpeg "$DEST/$file"
  done
  ssh "$MAC" "xcrun simctl shutdown $U >/dev/null 2>&1 || true"
done
ssh "$MAC" "rm -f /tmp/moonrise-store-shot.png /tmp/moonrise-store-shot.jpg"
echo "==> $OUT"
