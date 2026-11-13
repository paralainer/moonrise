#!/usr/bin/env bash
# Build the Android TV app and install it on a TV (adb, over the network or USB), then open it.
#
# Two apps, side by side on one TV, each with its own sign-in, settings and Watch Next rows:
#   - Moonrise (app.moonrise.tv), the release build: R8, signed with the release key. The default,
#     the app for watching. Needs MOONRISE_ANDROID_SIGNING (see clients/android/README.md).
#   - Moonrise Dev (app.moonrise.tv.dev), the debug build (DEV=1): for trying out changes and for
#     scripted tests (its test seams), without touching the release app or its sign-in.
#
#   scripts/deploy-android-tv.sh 192.168.1.50          # the TV's address (port 5555 unless given)
#   ANDROID_TV=192.168.1.50 scripts/deploy-android-tv.sh
#   DEV=1 scripts/deploy-android-tv.sh <tv>            # the Dev app
#   ANDROID_SERIAL=<serial> scripts/deploy-android-tv.sh   # a device the adb server already lists
#                                                      # (`adb devices`): a TV on USB, an emulator
#   ADB_PORT=5037 ANDROID_SERIAL=<serial> ...          # through the adb server on that port, e.g.
#                                                      # the computer's own, which sees USB devices
#                                                      # (default: clients/android/adb.sh's, 5038)
#   NO_BUILD=1 ...                                     # install the APK built last
#   NO_LAUNCH=1 ...                                    # don't open the app afterwards
#   APP=phone ANDROID_SERIAL=<serial> ...              # the phone app (app.moonrise.phone[.dev]) on a
#                                                      # phone or tablet, the same way
#
# Release builds come as one APK per CPU type: the one for the device's ABI list
# (`getprop ro.product.cpu.abilist`) is installed, else the universal one (scripts/android-apk.sh).
#
# Over the network, the TV needs developer options and network debugging on, and allows this
# computer once (it asks on screen at the first connection). Everything runs in Docker
# (clients/android/build.sh, adb.sh).
#
# Fails (exit 1) with adb's answer when the install doesn't happen: the TV is unreachable or
# hasn't allowed this computer, or the app on it is signed with another key. This script never
# uninstalls anything: uninstalling an app removes its sign-in.
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ANDROID="$DIR/clients/android"
ADB="$ANDROID/adb.sh"
export ADB_PORT="${ADB_PORT:-5038}"

# The TV first: a missing or unreachable target fails now, not after a full build.
if [ -n "${1:-${ANDROID_TV:-}}" ]; then
    TV="${1:-$ANDROID_TV}"
    case "$TV" in *:*) ;; *) TV="$TV:5555" ;; esac
    echo "==> connecting to $TV"
    out="$("$ADB" connect "$TV" 2>&1 || true)"
    echo "$out"
    case "$out" in
        *"connected to"*) ;;
        *) echo "error: can't connect to $TV (network debugging on? this computer allowed on the TV?)" >&2; exit 1 ;;
    esac
elif [ -n "${ANDROID_SERIAL:-}" ]; then
    TV="$ANDROID_SERIAL"
else
    echo "usage: $0 <tv-address[:port]>   (or ANDROID_TV=<address>, or ANDROID_SERIAL=<serial> for a device adb already lists)" >&2
    exit 2
fi
state="$("$ADB" -s "$TV" get-state 2>&1 || true)"
[ "$state" = device ] || { echo "error: $TV is '$state', not ready (adb server on port $ADB_PORT; plugged in, allowed on the TV's screen?)" >&2; exit 1; }

APP="${APP:-tv}"
case "$APP" in
    tv) BUILD_DEBUG=(); BUILD_RELEASE=(release) ;;
    phone) BUILD_DEBUG=(phone); BUILD_RELEASE=(phone-release) ;;
    *) echo "error: APP is tv or phone, not '$APP'" >&2; exit 2 ;;
esac
if [ "${DEV:-}" = 1 ]; then
    PACKAGE="app.moonrise.$APP.dev"; NAME="Moonrise Dev"
    APK="$ANDROID/$APP/build/outputs/apk/debug/$APP-debug.apk"
    [ "${NO_BUILD:-}" = 1 ] || "$ANDROID/build.sh" "${BUILD_DEBUG[@]}"
else
    PACKAGE="app.moonrise.$APP"; NAME="Moonrise"
    [ "${NO_BUILD:-}" = 1 ] || "$ANDROID/build.sh" "${BUILD_RELEASE[@]}"
    . "$DIR/scripts/android-apk.sh"
    abis="$("$ADB" -s "$TV" shell getprop ro.product.cpu.abilist 2>/dev/null | tr -d '\r' || true)"
    APK="$(apk_for_abis "$ANDROID/$APP/build/outputs/apk/release" "$APP" "$abis" || true)"
    [ -n "$APK" ] || { echo "error: no release APK for $TV (ABIs: ${abis:-unknown}) in $ANDROID/$APP/build/outputs/apk/release" >&2; exit 1; }
    echo "==> $TV runs ${abis:-unknown ABIs}: $(basename "$APK")"
fi
[ -f "$APK" ] || { echo "error: no APK at $APK" >&2; exit 1; }

updated() { "$ADB" -s "$TV" shell dumpsys package "$PACKAGE" 2>/dev/null | sed -n 's/^ *lastUpdateTime=//p' | head -1 || true; }
before="$(updated)"
echo "==> installing $NAME ($PACKAGE, $(basename "$APK")) on $TV"
set +e
out="$("$ADB" -s "$TV" install -r "$APK" 2>&1)"
code=$?
set -e
echo "$out" | tail -3
if [ $code -ne 0 ] || ! grep -q '^Success' <<<"$out"; then
    if grep -q 'INSTALL_FAILED_UPDATE_INCOMPATIBLE' <<<"$out"; then
        echo "error: $PACKAGE on the TV is signed with another key (app.moonrise.tv: a debug build from before the Dev app existed, or another release key; app.moonrise.tv.dev: another computer's debug key). Replacing it means uninstalling it first, which signs it out; this script never does." >&2
    fi
    echo "error: install failed on $TV" >&2
    exit 1
fi
after="$(updated)"
if [ -z "$after" ] || [ "$after" = "$before" ]; then
    echo "error: adb said Success, but the TV's copy of $PACKAGE didn't change (lastUpdateTime ${after:-none})" >&2
    exit 1
fi
echo "==> installed ($PACKAGE updated $after)"

if [ "${NO_LAUNCH:-}" != 1 ]; then
    # Both apps' activity keeps the release app's class name (the package differs).
    "$ADB" -s "$TV" shell am start -n "$PACKAGE/app.moonrise.$APP.MainActivity" >/dev/null
    echo "==> opened $NAME on $TV"
fi
