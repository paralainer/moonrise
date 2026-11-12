#!/usr/bin/env bash
# Drive the shared simulator with idb. Coordinates are screenshot pixels from shot.sh (portrait 603 wide = 1.5x points).
# Usage: tap.sh X Y | tap.sh land X Y (from a ROT=270 shot) | tap.sh swipe X1 Y1 X2 Y2 [dur] | tap.sh text "..."
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/common.sh"
SIM=${SIM:-$SIM_PHONE}
need_mac
need "$SIM" "no iPhone simulator: set SIM_PHONE in scripts/mac.env (a UDID from xcrun simctl list devices on the Mac; scripts/sim-create.sh makes one), or SIM=<udid>"
pt() { python3 -c "print(round($1/1.5))"; }
case "${1:-}" in
  land)  CMD="ui tap --udid $SIM --duration 0.1 $(pt $((603 - $3))) $(pt $2)" ;;
  swipe) CMD="ui swipe --udid $SIM --duration ${6:-0.4} $(pt $2) $(pt $3) $(pt $4) $(pt $5)" ;;
  text)  CMD="ui text --udid $SIM '$2'" ;;
  *)     CMD="ui tap --udid $SIM --duration 0.1 $(pt $1) $(pt $2)" ;;
esac
ssh "$MAC" "$MAC_PATH
pgrep -f 'idb_companion --udid $SIM' >/dev/null || { nohup idb_companion --udid $SIM --grpc-port 10882 >/tmp/idbc.log 2>&1 & sleep 3; }
~/idb-venv/bin/idb connect localhost 10882 >/dev/null 2>&1 || true
~/idb-venv/bin/idb $CMD"
