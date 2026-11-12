#!/usr/bin/env bash
# Siri Remote on the Apple TV simulator: scripts/tvkey.sh up|down|left|right|select|menu [count]
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/common.sh"
SIM=$SIM_TV
need_mac
need "$SIM" "no Apple TV simulator: set SIM_TV in scripts/mac.env (a UDID from xcrun simctl list devices on the Mac; scripts/sim-create.sh tv makes one)"
case "$1" in right) K=79;; left) K=80;; down) K=81;; up) K=82;; select) K=40;; menu) K=41;; *) echo "?"; exit 2;; esac
N=${2:-1}
ssh "$MAC" "$MAC_PATH
pgrep -f 'idb_companion --udid $SIM' >/dev/null || { nohup idb_companion --udid $SIM --grpc-port 10883 >/tmp/idbc-tv.log 2>&1 & sleep 4; }
~/idb-venv/bin/idb connect localhost 10883 >/dev/null 2>&1 || true
for i in \$(seq $N); do ~/idb-venv/bin/idb ui key --udid $SIM $K >/dev/null 2>&1; sleep 0.4; done"
