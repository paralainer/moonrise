#!/usr/bin/env bash
# Screenshot the shared simulator into the scratch dir. Usage: scripts/shot.sh name
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/common.sh"
need_mac
need "${SIM:-$SIM_PHONE}" "no iPhone simulator: set SIM_PHONE in scripts/mac.env (a UDID from xcrun simctl list devices on the Mac; scripts/sim-create.sh makes one), or SIM=<udid>"
OUT=${SHOT_DIR:-/tmp/moonrise-shots}
mkdir -p "$OUT"
ssh "$MAC" "xcrun simctl io ${SIM:-$SIM_PHONE} screenshot /tmp/moonrise-shot.png >/dev/null 2>&1 && sips -Z 1311 /tmp/moonrise-shot.png >/dev/null && { [ -n \"${ROT:-}\" ] && sips -r ${ROT:-0} /tmp/moonrise-shot.png >/dev/null || true; }"
scp -q "$MAC":/tmp/moonrise-shot.png "$OUT/$1.png"

echo "$OUT/$1.png"
