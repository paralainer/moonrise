#!/usr/bin/env bash
# Drive and read the iPhone app on a simulator for network tests (with scripts/netgate.py between
# the simulator and the server). Runs the simulator side over ssh on the build Mac (MAC, SIM_PHONE
# or SIM, DEV=1 for the Dev app, as the other scripts).
#
#   sim-net.sh install                 install the last sim build (scripts/build.sh sim), app data kept
#   sim-net.sh launch [--reset]        launch signed in (as scripts/sim.sh: MOONRISE_URL and
#                                      MOONRISE_CREDS_FILE), with the seams below; the app's output
#                                      goes to /tmp/moonrise-net-<sim>.log on the Mac
#   sim-net.sh relaunch                launch again with no seams (a user opening the app)
#   sim-net.sh terminate               stop the app (as iOS ending it)
#   sim-net.sh home | lock             press Home or the side button
#   sim-net.sh switch [bundle id]      open another app (default the Settings app): ours goes to the background
#   sim-net.sh front                   bring the app back to the front (as tapping its icon)
#   sim-net.sh idx                     its downloads, one line each: state, percent, segments, error
#   sim-net.sh idx-watch SECS [EVERY]  idx with a timestamp every EVERY seconds (default 5) for SECS
#   sim-net.sh ax [PATTERN]            the accessibility labels and values on screen (grep -E PATTERN)
#   sim-net.sh tap-label PATTERN       tap the first element whose label matches (grep -E)
#   sim-net.sh tap X Y                 tap there (points; a shot.png pixel / 1.5)
#   sim-net.sh swipe X1 Y1 X2 Y2       swipe (points)
#   sim-net.sh log [N] [PATTERN]       the app's last N output lines (default 40), optionally filtered
#   sim-net.sh shot NAME               screenshot to SHOT_DIR (default /tmp/moonrise-shots)/NAME.png
#   sim-net.sh files ID                the files of a download (by its record id) on the device
#
# Seams for launch (DebugSeams.swift): DL=<item>[:<bits/s>] PLAYDL=<item> DELDL=<item>
# AUTOPLAY=<item> AT= QUALITY= SEEK= PAUSE= GIVEUP= OPEN=<item> PIN=1 CELL=1 (pretend cellular)
# TSKEY_FILE=<chmod 600 file with a Tailscale auth key> (sign the embedded node in and turn it on)
# AWAY=1 (treat the LAN as unreachable). FAKE_TAILNET=<host:port> (that address stands in for the
# tailnet route: a second gate channel, no Tailscale needed). MANAGE=<screen>.
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/common.sh"
SIM=${SIM:-$SIM_PHONE}
need_mac
need "$SIM" "no iPhone simulator: set SIM_PHONE in scripts/mac.env or SIM=<udid> (scripts/sim-create.sh makes one)"
LOG=/tmp/moonrise-net-$SIM.log
cmd=${1:-}; shift || true

# The app's data container on the Mac (empty when it isn't installed).
container='$(xcrun simctl get_app_container '$SIM' '$APP_ID' data 2>/dev/null)'

idb_prelude="$MAC_PATH
pgrep -f 'idb_companion --udid $SIM' >/dev/null || { nohup idb_companion --udid $SIM --grpc-port ${IDB_PORT:-10892} >/tmp/idbc-$SIM.log 2>&1 & sleep 3; }
~/idb-venv/bin/idb connect localhost ${IDB_PORT:-10892} >/dev/null 2>&1 || true"

# index.json -> one line per download (python3 on the Mac).
idx_py='import json,sys
try: recs=json.load(open(sys.argv[1]))
except Exception as e: print("no downloads ("+str(e).split(":")[0]+")"); sys.exit(0)
for r in recs:
  st=r.get("state"); p=r.get("progress") or 0
  print("%-11s %5.1f%% seg %s/%s %s%s | %s%s" % (st, p*100, r.get("segmentsDone","-"), r.get("segmentCount","-"),
    r.get("qualityLabel",""), (" err="+r["error"]) if r.get("error") else "", r.get("title",""), " id="+r.get("id","")))'

case "$cmd" in
  install)
    ssh "$MAC" "xcrun simctl bootstatus $SIM -b >/dev/null 2>&1 || xcrun simctl boot $SIM; xcrun simctl install $SIM ${APP:-~/$MAC_DIR/build-sim$BUILD_SUFFIX/Build/Products/Debug-iphonesimulator/Moonrise.app} && echo installed" ;;
  launch|relaunch)
    U=; P=; K=
    if [ "$cmd" = launch ]; then
      if [ "${1:-}" = --reset ]; then ssh "$MAC" "xcrun simctl terminate $SIM $APP_ID 2>/dev/null; xcrun simctl uninstall $SIM $APP_ID 2>/dev/null; true"; "$0" install; fi
      if [ -n "${MOONRISE_URL:-}" ]; then
        need "${MOONRISE_CREDS_FILE:-}" "MOONRISE_URL needs MOONRISE_CREDS_FILE (a chmod 600 file: user name, then password)"
        { read -r U; read -r P; } < "$MOONRISE_CREDS_FILE"
      fi
      [ -n "${TSKEY_FILE:-}" ] && K=$(cat "$TSKEY_FILE")
    fi
    seams=
    if [ "$cmd" = launch ]; then
      for kv in "AUTODOWNLOAD=${DL:-}" "PLAY_DOWNLOAD=${PLAYDL:-}" "DELETE_DOWNLOAD=${DELDL:-}" "AUTOPLAY=${AUTOPLAY:-}" \
        "AUTOPLAY_AT=${AT:-}" "AUTOPLAY_QUALITY=${QUALITY:-}" "AUTOPLAY_SEEK=${SEEK:-}" "AUTOPLAY_PAUSE=${PAUSE:-}" \
        "RECOVERY_GIVEUP=${GIVEUP:-}" "OPEN_ITEM=${OPEN:-}" "PIN_CONTROLS=${PIN:-}" "FORCE_CELLULAR=${CELL:-}" \
        "FORCE_TAILNET=${AWAY:-}" "FAKE_TAILNET=${FAKE_TAILNET:-}" "MANAGE=${MANAGE:-}"; do
        seams+=" SIMCTL_CHILD_MOONRISE_$kv"
      done
    fi
    printf '%s\n%s\n%s\n' "$U" "$P" "$K" | ssh "$MAC" "read -r U; read -r P; read -r K
SIM=$SIM; $(sim_launch "$LOG")
boot
xcrun simctl terminate \$SIM $APP_ID 2>/dev/null || true
export SIMCTL_CHILD_MOONRISE_NO_REPORT=1 $seams
[ -n \"\$K\" ] && export SIMCTL_CHILD_MOONRISE_TS_AUTHKEY=\"\$K\"
[ -n \"\$U\" ] && export SIMCTL_CHILD_MOONRISE_URL='${MOONRISE_URL:-}' SIMCTL_CHILD_MOONRISE_USER=\"\$U\" SIMCTL_CHILD_MOONRISE_PASS=\"\$P\"
start" ;;
  terminate) ssh "$MAC" "xcrun simctl terminate $SIM $APP_ID && echo terminated" ;;
  home) ssh "$MAC" "$idb_prelude
~/idb-venv/bin/idb ui button --udid $SIM HOME" ;;
  lock) ssh "$MAC" "$idb_prelude
~/idb-venv/bin/idb ui button --udid $SIM LOCK" ;;
  switch) ssh "$MAC" "xcrun simctl launch $SIM ${1:-com.apple.Preferences} >/dev/null && echo switched" ;;
  front) ssh "$MAC" "$idb_prelude
~/idb-venv/bin/idb launch --udid $SIM $APP_ID >/dev/null 2>&1 || xcrun simctl launch $SIM $APP_ID >/dev/null; echo front" ;;
  idx) ssh "$MAC" "f=$container/Library/Application\ Support/Downloads/index.json; python3 -c '$idx_py' \"\$f\"" ;;
  idx-watch)
    end=$(( $(date +%s) + ${1:?seconds} ))
    while [ "$(date +%s)" -lt "$end" ]; do
      "$0" idx | sed "s/^/$(date +%H:%M:%S) /"
      sleep "${2:-5}"
    done ;;
  ax)
    ssh "$MAC" "$idb_prelude
~/idb-venv/bin/idb ui describe-all --udid $SIM --json 2>/dev/null" | python3 -c '
import json,re,sys
pat=sys.argv[1] if len(sys.argv)>1 else ""
try: items=json.load(sys.stdin)
except Exception: items=[]
for e in items:
  t=" | ".join(x for x in (e.get("AXLabel"), e.get("AXValue")) if x)
  if t and (not pat or re.search(pat,t)):
    f=e.get("frame") or {}
    print("%s  [%s @%d,%d]" % (t, e.get("type",""), f.get("x",0)+f.get("width",0)/2, f.get("y",0)+f.get("height",0)/2))
' "${1:-}" ;;
  tap-label)
    pt=$("$0" ax "${1:?pattern}" | head -1 | sed -n 's/.*@\([0-9]*\),\([0-9]*\)]$/\1 \2/p')
    need "$pt" "nothing on screen matches $1"
    ssh "$MAC" "$idb_prelude
~/idb-venv/bin/idb ui tap --udid $SIM --duration 0.1 $pt && echo tapped $pt" ;;
  tap) ssh "$MAC" "$idb_prelude
~/idb-venv/bin/idb ui tap --udid $SIM --duration 0.1 ${1:?x} ${2:?y}" ;;
  swipe) ssh "$MAC" "$idb_prelude
~/idb-venv/bin/idb ui swipe --udid $SIM --duration 0.4 ${1:?} ${2:?} ${3:?} ${4:?}" ;;
  log) ssh "$MAC" "LC_ALL=C tr -cd '[:print:]\n' < $LOG | { [ -n '${2:-}' ] && grep -E '${2:-}' || cat; } | tail -n ${1:-40}" ;;
  shot)
    OUT=${SHOT_DIR:-/tmp/moonrise-shots}; mkdir -p "$OUT"
    ssh "$MAC" "xcrun simctl io $SIM screenshot /tmp/moonrise-net-shot-$SIM.png >/dev/null 2>&1 && sips -Z 1311 /tmp/moonrise-net-shot-$SIM.png >/dev/null"
    scp -q "$MAC:/tmp/moonrise-net-shot-$SIM.png" "$OUT/${1:?name}.png" && echo "$OUT/$1.png" ;;
  files) ssh "$MAC" "bash -s" <<EOF
d="$container/Library/Application Support/Downloads/${1:?record id}/hls"
for s in "\$d"/video "\$d"/audio/* "\$d"/subs/*; do [ -d "\$s" ] && echo "\${s#\$d/}: \$(ls "\$s" | wc -l | tr -d ' ') files, \$(du -sh "\$s" | cut -f1)"; done; true
EOF
    ;;
  -h|--help|"") awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }' "$0" ;;
  *) echo "unknown command: $cmd (sim-net.sh --help)" >&2; exit 2 ;;
esac
