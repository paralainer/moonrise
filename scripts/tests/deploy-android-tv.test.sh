#!/usr/bin/env bash
# scripts/deploy-android-tv.sh checks the TV (an address or ANDROID_SERIAL, connected and
# ready) before it builds, so a missing or unreachable target fails at once rather than after a full
# release build. clients/android/build.sh and adb.sh are stubs that log what they're asked
# (STUB_CONNECT=fail: the TV doesn't answer; STUB_STATE: what get-state says).
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
C=$(fresh_clone)
A="$C/clients/android"
mkdir -p "$A"
LOG="$SCRATCH/calls.log"
cat > "$A/build.sh" <<'STUB'
#!/bin/bash
echo "build ${*:-debug}" >> "$LOG"
d="$(dirname "$0")/tv/build/outputs/apk"
mkdir -p "$d/debug" "$d/release"; touch "$d/debug/tv-debug.apk" "$d/release/tv-release.apk"
STUB
cat > "$A/adb.sh" <<'STUB'
#!/bin/bash
echo "adb $*" >> "$LOG"
[ "$1" = -s ] && shift 2
case "$1" in
  connect) [ "${STUB_CONNECT:-}" = fail ] && echo "failed to connect to '$2': No route to host" || echo "connected to $2" ;;
  get-state) echo "${STUB_STATE:-device}" ;;
  install) touch "$LOG.installed"; echo "Performing Streamed Install"; echo Success ;;
  shell) case "$*" in
      *dumpsys*) [ -f "$LOG.installed" ] && echo "    lastUpdateTime=2026-11-08 10:00:01" || echo "    lastUpdateTime=2026-11-01 09:00:00" ;;
    esac ;;
esac
exit 0
STUB
chmod +x "$A/build.sh" "$A/adb.sh"
export LOG
unset ANDROID_TV ANDROID_SERIAL DEV NO_BUILD NO_LAUNCH STUB_CONNECT STUB_STATE
# deploy [address]: the real script, with the environment the call gives (VAR=... deploy).
deploy() {
  rm -f "$LOG" "$LOG.installed"; touch "$LOG"
  bash "$C/scripts/deploy-android-tv.sh" "$@" 2>&1
}
built() { grep -c '^build' "$LOG"; }

out=$(deploy); rc=$?
expect_status 2 $rc "no target" "$out"
expect_in "usage:" "$out" "prints the usage"
[ "$(built)" = 0 ] && ok || fail "no target: it built anyway" "$(cat "$LOG")"

out=$(STUB_CONNECT=fail deploy 192.0.2.1); rc=$?
expect_status 1 $rc "an address that doesn't answer" "$out"
expect_in "can't connect to 192.0.2.1:5555" "$out" "names the address, with the default port"
[ "$(built)" = 0 ] && ok || fail "unreachable TV: it built anyway" "$(cat "$LOG")"

out=$(ANDROID_SERIAL=emulator-5554 STUB_STATE=offline deploy); rc=$?
expect_status 1 $rc "a serial that isn't ready" "$out"
expect_in "emulator-5554 is 'offline'" "$out" "says what adb said"
[ "$(built)" = 0 ] && ok || fail "offline serial: it built anyway" "$(cat "$LOG")"

out=$(ANDROID_TV=192.0.2.1:5556 NO_LAUNCH=1 deploy); rc=$?
expect_status 0 $rc "a reachable TV" "$out"
expect_in "build release" "$(cat "$LOG")" "the default is the release build"
first=$(grep -nm1 '^adb connect' "$LOG" | cut -d: -f1); b=$(grep -nm1 '^build' "$LOG" | cut -d: -f1)
[ -n "$first" ] && [ -n "$b" ] && [ "$first" -lt "$b" ] && ok || fail "the TV wasn't checked before the build" "$(cat "$LOG")"
expect_in "installed (app.moonrise.tv updated" "$out" "the release app is installed"

out=$(DEV=1 ANDROID_SERIAL=emulator-5554 NO_LAUNCH=1 deploy); rc=$?
expect_status 0 $rc "DEV=1 on a serial" "$out"
expect_in "build debug" "$(cat "$LOG")" "DEV=1 builds the debug build"
expect_in "installed (app.moonrise.tv.dev updated" "$out" "the Dev app is installed"
s=$(grep -nm1 'get-state' "$LOG" | cut -d: -f1); b=$(grep -nm1 '^build' "$LOG" | cut -d: -f1)
[ -n "$s" ] && [ -n "$b" ] && [ "$s" -lt "$b" ] && ok || fail "the serial wasn't checked before the build" "$(cat "$LOG")"
done_testing
