#!/usr/bin/env bash
# scripts/deploy-android-tv.sh checks the TV (an address or ANDROID_SERIAL, connected and
# ready) before it builds, so a missing or unreachable target fails at once rather than after a full
# release build. A release build is one APK per CPU type, and the device gets the one for
# the first ABI it lists, else the universal one; APP=phone does the same for the phone app.
# clients/android/build.sh and adb.sh are stubs that log what they're asked (STUB_CONNECT=fail: the
# TV doesn't answer; STUB_STATE: what get-state says; STUB_ABIS: the device's ABI list).
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
C=$(fresh_clone)
A="$C/clients/android"
mkdir -p "$A"
LOG="$SCRATCH/calls.log"
cat > "$A/build.sh" <<'STUB'
#!/bin/bash
echo "build ${*:-debug}" >> "$LOG"
for app in tv phone; do
  d="$(dirname "$0")/$app/build/outputs/apk"
  mkdir -p "$d/debug" "$d/release"; touch "$d/debug/$app-debug.apk"
  for abi in armeabi-v7a arm64-v8a x86_64 universal; do touch "$d/release/$app-$abi-release.apk"; done
done
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
      *getprop\ ro.product.cpu.abilist*) printf '%s\r\n' "${STUB_ABIS-arm64-v8a,armeabi-v7a,armeabi}" ;;
      *dumpsys*) [ -f "$LOG.installed" ] && echo "    lastUpdateTime=2026-11-08 10:00:01" || echo "    lastUpdateTime=2026-11-01 09:00:00" ;;
    esac ;;
esac
exit 0
STUB
chmod +x "$A/build.sh" "$A/adb.sh"
export LOG
unset ANDROID_TV ANDROID_SERIAL DEV NO_BUILD NO_LAUNCH STUB_CONNECT STUB_STATE STUB_ABIS APP
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
installed() { grep '^adb .* install ' "$LOG" | sed 's/.* //'; }

out=$(ANDROID_SERIAL=tv-1 NO_LAUNCH=1 deploy); rc=$?
expect_status 0 $rc "a 64-bit TV" "$out"
expect_in "tv-arm64-v8a-release.apk" "$(installed)" "an arm64 device gets the arm64 APK"

out=$(ANDROID_SERIAL=tv-1 NO_LAUNCH=1 STUB_ABIS=armeabi-v7a,armeabi deploy); rc=$?
expect_status 0 $rc "a TV with a 32-bit system" "$out"
expect_in "tv-armeabi-v7a-release.apk" "$(installed)" "a 32-bit system gets the 32-bit APK"

out=$(ANDROID_SERIAL=tv-1 NO_LAUNCH=1 STUB_ABIS=x86 deploy); rc=$?
expect_status 0 $rc "an ABI with no APK of its own" "$out"
expect_in "tv-universal-release.apk" "$(installed)" "anything else gets the universal APK"

out=$(ANDROID_SERIAL=tv-1 NO_LAUNCH=1 STUB_ABIS= deploy); rc=$?
expect_status 0 $rc "a device that doesn't say" "$out"
expect_in "tv-universal-release.apk" "$(installed)" "no ABI list: the universal APK"

out=$(APP=phone ANDROID_SERIAL=phone-1 deploy); rc=$?
expect_status 0 $rc "the phone app" "$out"
expect_in "build phone-release" "$(cat "$LOG")" "APP=phone builds the phone's release"
expect_in "phone-arm64-v8a-release.apk" "$(installed)" "the phone gets its own app's APK for its ABI"
expect_in "am start -n app.moonrise.phone/app.moonrise.phone.MainActivity" "$(cat "$LOG")" "and opens the phone app"

out=$(APP=phone DEV=1 ANDROID_SERIAL=phone-1 NO_LAUNCH=1 deploy); rc=$?
expect_status 0 $rc "the phone's Dev app" "$out"
expect_in "build phone" "$(cat "$LOG")" "DEV=1 APP=phone builds the phone's debug build"
expect_in "phone-debug.apk" "$(installed)" "and installs it"

out=$(APP=watch ANDROID_SERIAL=tv-1 deploy); rc=$?
expect_status 2 $rc "an unknown app" "$out"

# The picker on its own.
. "$C/scripts/android-apk.sh"
R="$C/clients/android/tv/build/outputs/apk/release"
expect_in "tv-x86_64-release.apk" "$(apk_for_abis "$R" tv " x86_64 , x86")" "spaces in the list don't matter"
rm -f "$R"/*universal*
apk_for_abis "$R" tv mips >/dev/null; expect_status 1 $? "no APK for the device and no universal one"
done_testing
