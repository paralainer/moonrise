#!/usr/bin/env bash
# Which release APK to install on an Android device: release builds are split by CPU type
# (clients/android/{tv,phone}/build.gradle.kts), so a device gets the APK of the first ABI it lists
# (`getprop ro.product.cpu.abilist`, its preferred first: a TV with a 64-bit CPU but a 32-bit
# system lists armeabi-v7a first), else the universal one.
#
#   . scripts/android-apk.sh; apk_for_abis <apk dir> <app: tv|phone> <abi list, comma-separated>
#   scripts/android-apk.sh <apk dir> <app> <abi list>      # the same, as a command
#
# Prints the APK's path, or nothing (exit 1) when the folder has none for the device.
apk_for_abis() {
    local dir="$1" app="$2" abis="$3" abi
    for abi in ${abis//,/ }; do
        abi="${abi//[[:space:]]/}"
        [ -n "$abi" ] && [ -f "$dir/$app-$abi-release.apk" ] && { echo "$dir/$app-$abi-release.apk"; return 0; }
    done
    [ -f "$dir/$app-universal-release.apk" ] && { echo "$dir/$app-universal-release.apk"; return 0; }
    return 1
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    [ $# -eq 3 ] || { echo "usage: $0 <apk dir> <tv|phone> <abi list>" >&2; exit 2; }
    apk_for_abis "$@"
fi
