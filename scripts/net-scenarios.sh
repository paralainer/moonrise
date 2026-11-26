#!/usr/bin/env bash
# The automated network scenarios of the iPhone app's downloads: the Dev app on a simulator
# (scripts/sim-net.sh), a network gate between it and a test server (scripts/netgate.py), a fault
# at a set point of a download, and a verdict from the app's own download index (state, percent,
# segments) sampled every few seconds. Use a throwaway server: some scenarios restart it.
#
#   net-scenarios.sh list
#   net-scenarios.sh run ID [ID ...]      e.g. run D05 D06
#   net-scenarios.sh all
#
# Environment (besides what sim-net.sh needs: MAC, SIM or SIM_PHONE, DEV=1, MOONRISE_URL = the
# gate's address, MOONRISE_CREDS_FILE):
#   GATE            the gate's control command, e.g. "python3 scripts/netgate.py ctl --control 127.0.0.1:8790"
#   ITEM            a film of about 10 minutes or more (its item id)
#   SHORT           three short items (ids, space-separated) for the queue scenarios
#   SUBS_ITEM       a title with many subtitle tracks (D14; default ITEM)
#   SERVER_RESTART  a command that restarts the server (D17); SERVER_STOP and SERVER_START stop
#                   and start it (D18)
#   QUALITY         bits/s of the downloads (default 1500000, 480p: a transcode); RATE the gate's
#                   rate during a run (default 600k, so a download takes a few minutes)
#   OUT             where each scenario's timeline goes (default /tmp/moonrise-net-results)
#
# Each scenario prints "<ID> PASS|FAIL <why>" and writes OUT/<ID>.log: the index samples, the
# gate's settings and the app's download and route log lines. Exit status: the number of FAILs.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SN="$ROOT/scripts/sim-net.sh"
OUT=${OUT:-/tmp/moonrise-net-results}
QUALITY=${QUALITY:-1500000}
RATE=${RATE:-600k}
POLL=${POLL:-3}
mkdir -p "$OUT"

gate() { $GATE "$@" >/dev/null; }

# -- sampling -------------------------------------------------------------------------------------
# The newest download's index line (state percent "seg" done/count ...), or the line matching $1.
rec() { if [ -n "${1:-}" ]; then "$SN" idx | grep -F -- "$1" | head -1; else "$SN" idx | head -1; fi; }
STATE=; PCT=0; SEG=; MAXPCT=0; FAILED_SEEN=0; REGRESSED=0; WAITING_SEEN=0
sample() {
  local line; line=$(rec "${1:-}")
  echo "$(date +%T) $line" >> "$LOG"
  STATE=$(awk '{print $1}' <<<"$line")
  PCT=$(awk '{sub("%","",$2); print $2+0}' <<<"$line")
  SEG=$(awk '{print $4}' <<<"$line")
  [ "$STATE" = failed ] && FAILED_SEEN=1
  grep -q "err=Waiting" <<<"$line" && WAITING_SEEN=1
  if awk -v p="$PCT" -v m="$MAXPCT" 'BEGIN { exit !(p + 0.5 < m) }'; then REGRESSED=1; fi
  if awk -v p="$PCT" -v m="$MAXPCT" 'BEGIN { exit !(p > m) }'; then MAXPCT=$PCT; fi
}
# wait_for <seconds> <condition...>: sample until the condition holds (0) or the time runs out (1).
wait_for() {
  local end=$(( $(date +%s) + $1 )); shift
  while [ "$(date +%s)" -lt "$end" ]; do
    sample "${MATCH:-}"
    "$@" && return 0
    sleep "$POLL"
  done
  return 1
}
pct_ge() { awk -v p="$PCT" -v n="$1" 'BEGIN { exit !(p >= n) }'; }
pct_gt() { awk -v p="$PCT" -v n="$1" 'BEGIN { exit !(p > n) }'; }
state_is() { [ "$STATE" = "$1" ]; }
moved_past() { pct_gt "$1" || state_is completed; }
# hold <seconds>: keep sampling for that long.
hold() { local end=$(( $(date +%s) + $1 )); while [ "$(date +%s)" -lt "$end" ]; do sample "${MATCH:-}"; sleep "$POLL"; done; }

begin() {
  ID=$1; LOG="$OUT/$ID.log"; : > "$LOG"
  STATE=; PCT=0; SEG=; MAXPCT=0; FAILED_SEEN=0; REGRESSED=0; WAITING_SEEN=0; MATCH=
  echo "== $ID $(date '+%F %T') ${2:-}" >> "$LOG"
  gate open; gate set off
}
# start_download <item> [rate]: a fresh app (no downloads), signed in, downloading the item.
start_download() {
  gate set "rate=${2:-$RATE}"
  DL="$1:$QUALITY" "$SN" launch --reset >> "$LOG" 2>&1
}
verdict() { # verdict PASS|FAIL <why>
  { echo "-- gate:"; $GATE state; echo "-- app log:"; "$SN" log 60 'MoonriseDownload|MoonriseRouter'; } >> "$LOG" 2>&1
  echo "== $1 $2" >> "$LOG"
  echo "$ID $1 $2"
  [ "$1" = PASS ]
}
# finish <seconds>: wait for completed; then the common checks.
finish() {
  if ! wait_for "$1" state_is completed; then verdict FAIL "not completed in $1 s (state $STATE, $PCT %, seg $SEG)"; return; fi
  [ "$FAILED_SEEN" = 1 ] && { verdict FAIL "went to failed on the way (recovered later)"; return; }
  [ "$REGRESSED" = 1 ] && { verdict FAIL "percent went backwards"; return; }
  local done count; done=${SEG%/*}; count=${SEG#*/}
  [ "$done" = "$count" ] || { verdict FAIL "completed with $SEG segments"; return; }
  verdict PASS "${NOTE:-} completed, $SEG segments"
}
# resumed_within <seconds>: after a fault ends, the percent moves past where it was.
resumed_within() {
  local at=$PCT t0; t0=$(date +%s)
  # Finished while away (the simulator lets a backgrounded app run on): nothing to resume.
  [ "$STATE" = completed ] && { NOTE="${NOTE:-}completed while away;"; return 0; }
  if wait_for "$1" moved_past "$at"; then NOTE="${NOTE:-}resumed $(( $(date +%s) - t0 )) s after the network came back;"; return 0; fi
  verdict FAIL "no progress for $1 s after the network came back (state $STATE, $PCT %)"; return 1
}

# -- scenarios ------------------------------------------------------------------------------------
# fault <ID> <description> <percent to strike at> <fault command> <seconds> [restore command]
fault_scenario() {
  local id=$1 what=$2 at=$3 fault=$4 secs=$5 restore=${6:-"gate open"}
  begin "$id" "$what"; NOTE=
  start_download "$ITEM"
  wait_for 300 pct_ge "$at" || { verdict FAIL "never reached $at % (state $STATE, $PCT %)"; return; }
  echo "-- fault: $fault for $secs s" >> "$LOG"
  eval "$fault"
  hold "$secs"
  [ "$STATE" = failed ] && { verdict FAIL "failed during the outage"; return; }
  echo "-- restore: $restore" >> "$LOG"
  eval "$restore"
  resumed_within "${RESUME_LIMIT:-45}" || return
  finish 900
}

s_D01() { begin D01 "clean link, transcoded"; NOTE=; start_download "$ITEM" 0
  local t0; t0=$(date +%s)
  wait_for 60 pct_gt 0 || { verdict FAIL "percent still 0 after 60 s"; return; }
  NOTE="first percent after $(( $(date +%s) - t0 )) s;"; finish 600; }
s_D02() { begin D02 "clean link, Original"; NOTE=; local q=$QUALITY; QUALITY=0; start_download "$ITEM" 0; QUALITY=$q
  wait_for 60 pct_gt 0 || { verdict FAIL "percent still 0 after 60 s"; return; }; finish 600; }
s_D05() { fault_scenario D05 "cut 5 s at 20 %" 20 "gate cut" 5; }
s_D06() { fault_scenario D06 "cut 3 min at 20 %" 20 "gate cut" 180; }
s_D07() { fault_scenario D07 "stall 2 min at 20 % (dead link)" 20 "gate stall" 120; }
s_D08() { begin D08 "every connection reset after 2 MB"; NOTE=; gate set drop_after=2m; start_download "$ITEM"; finish 900; }
s_D09() { begin D09 "random resets (2 % of chunks)"; NOTE=; gate set reset=0.02; start_download "$ITEM"; finish 900; }
s_D10() { begin D10 "slow link, 300 kB/s"; NOTE=; start_download "$ITEM" 300k
  local t0; t0=$(date +%s)
  wait_for 90 pct_gt 0 || { verdict FAIL "percent still 0 after 90 s on a slow link"; return; }
  NOTE="first percent after $(( $(date +%s) - t0 )) s;"; finish 1500; }
s_D11() { begin D11 "latency 300 ms + jitter 300 ms"; NOTE=; gate set delay=300 jitter=300; start_download "$ITEM"; finish 1200; }
s_D12() { begin D12 "lossy: 10 % of chunks stall 0.2-1.5 s"; NOTE=; gate set loss=0.1; start_download "$ITEM"; finish 1200; }
s_D13() { begin D13 "flapping: 10 s up, 5 s down, 6 times, from 10 %"; NOTE=; start_download "$ITEM"
  wait_for 300 pct_ge 10 || { verdict FAIL "never reached 10 %"; return; }
  $GATE flap 10 5 6 >/dev/null; hold 95; finish 900; }
s_D14() { begin D14 "many subtitle tracks"; NOTE=; start_download "${SUBS_ITEM:-$ITEM}" 0
  wait_for 60 pct_gt 0 || { verdict FAIL "percent still 0 after 60 s"; return; }
  finish 900; "$SN" files "$(rec | sed -n 's/.* id=//p')" >> "$LOG"; }
s_D17() { begin D17 "server restart at 30 %"; NOTE=; start_download "$ITEM"
  wait_for 300 pct_ge 30 || { verdict FAIL "never reached 30 %"; return; }
  echo "-- restarting the server" >> "$LOG"; eval "$SERVER_RESTART" >> "$LOG" 2>&1
  resumed_within 120 || return; finish 900; }
s_D18() { fault_scenario D18 "server stopped for 5 min at 20 %" 20 'eval "$SERVER_STOP" >/dev/null' 300 'eval "$SERVER_START" >/dev/null'; }
s_D16() { begin D16 "three downloads queued, 60 s cut while the first runs"; NOTE=
  gate set "rate=$RATE"
  set -- $SHORT
  DL="$1:$QUALITY" "$SN" launch --reset >> "$LOG" 2>&1; sleep 8
  DL="$2:$QUALITY" "$SN" launch >> "$LOG" 2>&1; sleep 8
  DL="$3:$QUALITY" "$SN" launch >> "$LOG" 2>&1; sleep 10
  echo "-- fault: cut 60 s" >> "$LOG"; gate cut
  local end=$(( $(date +%s) + 60 )) lines
  while [ "$(date +%s)" -lt "$end" ]; do
    lines=$("$SN" idx); printf '%s\n' "$lines" | sed "s/^/$(date +%T) /" >> "$LOG"
    grep -q '^failed' <<<"$lines" && { gate open; verdict FAIL "one failed during the cut"; return; }
    [ "$(grep -c '^downloading' <<<"$lines")" -gt 1 ] && { gate open; verdict FAIL "several running at once during the cut"; return; }
    sleep "$POLL"
  done
  echo "-- restore: open" >> "$LOG"; gate open
  end=$(( $(date +%s) + 900 ))
  while [ "$(date +%s)" -lt "$end" ]; do
    lines=$("$SN" idx); printf '%s\n' "$lines" | sed "s/^/$(date +%T) /" >> "$LOG"
    grep -q '^failed' <<<"$lines" && { verdict FAIL "one failed"; return; }
    [ "$(grep -c '^completed' <<<"$lines")" = 3 ] && { verdict PASS "all three completed after the cut, one at a time"; return; }
    sleep "$POLL"
  done
  verdict FAIL "not all completed in 900 s after the cut"; }
# Lifecycle: the app in the background, switched away, locked, ended.
lifecycle() { # lifecycle <ID> <what> <action before> <seconds away> <action after>
  begin "$1" "$2"; NOTE=; start_download "$ITEM"
  wait_for 300 pct_ge 20 || { verdict FAIL "never reached 20 %"; return; }
  local before=$PCT; eval "$3" >> "$LOG" 2>&1; hold "$4"; local away=$PCT; eval "$5" >> "$LOG" 2>&1
  NOTE="$before % -> $away % while away;"
  resumed_within 60 || return; finish 900; }
s_L01() { lifecycle L01 "Home for 10 s" '"$SN" home' 10 '"$SN" front'; }
s_L02() { lifecycle L02 "Home for 5 min" '"$SN" home' 300 '"$SN" front'; }
s_L03() { lifecycle L03 "another app for 60 s" '"$SN" switch' 60 '"$SN" front'; }
s_L05() { lifecycle L05 "locked for 2 min" '"$SN" lock' 120 '"$SN" home; sleep 2; "$SN" front'; }
s_L06() { lifecycle L06 "app ended at 20 %, opened again 30 s later" '"$SN" terminate' 30 '"$SN" relaunch'; }
s_L07() { lifecycle L07 "server cut, app ended, opened again during the outage, then the server is back" \
  'gate cut; sleep 5; "$SN" terminate; sleep 5; "$SN" relaunch' 60 'gate open'; }
s_L10() { lifecycle L10 "in the background during a 60 s cut, back after it heals" \
  '"$SN" home; sleep 3; gate cut' 60 'gate open; sleep 5; "$SN" front'; }
s_D15() { begin D15 "three downloads queued"; NOTE=
  gate set "rate=$RATE"
  set -- $SHORT
  DL="$1:$QUALITY" "$SN" launch --reset >> "$LOG" 2>&1; sleep 8
  DL="$2:$QUALITY" "$SN" launch >> "$LOG" 2>&1; sleep 8
  DL="$3:$QUALITY" "$SN" launch >> "$LOG" 2>&1
  local end=$(( $(date +%s) + 900 )) lines running
  while [ "$(date +%s)" -lt "$end" ]; do
    lines=$("$SN" idx); printf '%s\n' "$lines" | sed "s/^/$(date +%T) /" >> "$LOG"
    running=$(grep -c '^downloading' <<<"$lines")
    [ "$running" -gt 1 ] && { verdict FAIL "$running downloads running at once"; return; }
    grep -q '^failed' <<<"$lines" && { verdict FAIL "one failed"; return; }
    [ "$(grep -c '^completed' <<<"$lines")" = 3 ] && { verdict PASS "all three completed, one at a time"; return; }
    sleep "$POLL"
  done
  verdict FAIL "not all completed in 900 s"; }

# -- streaming ------------------------------------------------------------------------------------
# The app's player log (MoonriseTiming lines) since the scenario's launch.
plog() { "$SN" log 400 'MoonriseTiming|MoonriseRouter'; }
# stream_scenario <ID> <what> <fault> <seconds> <restore> <expect: recover|ride|giveup>
#   recover: a recovery starts and ends with "recovered at"; ride: playback rides it out (no
#   recovery needed, or one that ends recovered); giveup: it gives up with a message.
stream_scenario() {
  local id=$1 what=$2 fault=$3 secs=$4 restore=$5 expect=$6
  begin "$id" "$what"; NOTE=
  [ -n "${SRATE:-}" ] && gate set "rate=$SRATE"
  AUTOPLAY="$ITEM" GIVEUP="${GIVEUP:-60}" PIN=1 "$SN" launch --reset >> "$LOG" 2>&1
  local i=0
  until plog | grep -q 'engine ready\|playing at'; do i=$((i + 1)); [ $i -gt 40 ] && { verdict FAIL "never started playing"; return; }; sleep 3; done
  sleep 20
  echo "-- fault: $fault for $secs s" >> "$LOG"
  eval "$fault"; sleep "$secs"
  echo "-- restore: $restore" >> "$LOG"
  eval "$restore"
  sleep "${AFTER:-60}"
  "$SN" shot "$id" >> "$LOG" 2>&1
  local p; p=$(plog); printf '%s\n' "$p" | tail -40 >> "$LOG"
  local recovered gaveup started
  started=$(grep -c 'recovery: ' <<<"$p"); recovered=$(grep -c 'recovered at' <<<"$p"); gaveup=$(grep -c 'gave up' <<<"$p")
  case "$expect" in
    recover) [ "$recovered" -gt 0 ] && [ "$gaveup" = 0 ] && verdict PASS "recovered ($recovered recoveries)" || verdict FAIL "recovery: started $started, recovered $recovered, gave up $gaveup" ;;
    ride) [ "$gaveup" = 0 ] && { [ "$started" = 0 ] || [ "$recovered" -gt 0 ]; } && verdict PASS "rode it out (recoveries: $recovered)" || verdict FAIL "recovery: started $started, recovered $recovered, gave up $gaveup" ;;
    giveup) [ "$gaveup" -gt 0 ] && verdict PASS "gave up with a message, as designed" || verdict FAIL "didn't give up (started $started, recovered $recovered)" ;;
  esac
}
s_S05() { stream_scenario S05 "cut 5 s during playback" "gate cut" 5 "gate open" ride; }
s_S06() { GIVEUP=60 AFTER=20 stream_scenario S06 "cut 100 s during playback (gives up at 60 s)" "gate cut" 100 "gate open" giveup; }
s_S07() { stream_scenario S07 "cut 40 s during playback" "gate cut" 40 "gate open" recover; }
s_S08() { stream_scenario S08 "stall 40 s during playback (dead link)" "gate stall" 40 "gate open" recover; }
s_S12() { stream_scenario S12 "server restart during playback" "eval \"\$SERVER_RESTART\" >/dev/null" 1 ":" recover; }
s_S13() { stream_scenario S13 "every connection reset after 1 MB for 60 s" "gate set drop_after=1m" 60 "gate set drop_after=0" ride; }
s_S14() { stream_scenario S14 "flapping: 8 s up, 4 s down, 5 times" "\$GATE flap 8 4 5 >/dev/null" 60 ":" ride; }

LIST="D01 D02 D05 D06 D07 D08 D09 D10 D11 D12 D13 D14 D15 D16 D17 D18 L01 L02 L03 L05 L06 L07 L10 S05 S06 S07 S08 S12 S13 S14"
case "${1:-}" in
  list) echo "$LIST" ;;
  run) shift; fails=0; for id in "$@"; do "s_$id" || fails=$((fails + 1)); done; gate open; gate set off; exit $fails ;;
  all) fails=0; for id in $LIST; do "s_$id" || fails=$((fails + 1)); done; gate open; gate set off; exit $fails ;;
  *) awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }' "$0"; [ -n "${1:-}" ] && exit 2 || exit 0 ;;
esac
