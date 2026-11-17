#!/usr/bin/env bash
# Background modes: App Review refuses an Apple TV app that declares background audio it doesn't
# play. The app has one Info.plist for every platform, with audio, fetch and processing for iOS;
# project.yml's "Apple TV: no background modes" build phase drops them (and the background task
# ids) from the tvOS build only. Checked here:
# - background_modes_ok (scripts/common.sh), which scripts/check.sh runs on the simulator builds
#   and scripts/release-apple.sh on the store archives: an Apple TV plist with any background mode
#   fails, one without passes; an iOS plist passes only with audio, fetch and processing.
# - project.yml has the phase, after the Info.plist is made, and it acts on the Apple TV only.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
C=$(fresh_clone)
. "$C/scripts/common.sh" 2>/dev/null

plist() { printf '<?xml version="1.0" encoding="UTF-8"?>\n<plist version="1.0">\n<dict>\n\t<key>CFBundleName</key>\n\t<string>Moonrise</string>\n%s\t<key>NSBonjourServices</key>\n\t<array>\n\t\t<string>_moonrise._tcp</string>\n\t</array>\n</dict>\n</plist>\n' "$1"; }
modes() { local m out=$'\t<key>UIBackgroundModes</key>\n\t<array>\n'; for m in "$@"; do out+=$'\t\t<string>'"$m"$'</string>\n'; done; printf '%s\t</array>\n' "$out"; }
IOS=$(plist "$(modes audio fetch processing)")
NONE=$(plist '')

out=$(background_modes_ok tvos "$NONE" 2>&1); expect_status 0 $? "an Apple TV app without background modes" "$out"
out=$(background_modes_ok tvos "$IOS" 2>&1); expect_status 1 $? "an Apple TV app with iOS's background modes" "$out"
expect_in "declares background modes (audio fetch processing)" "$out" "names them"
out=$(background_modes_ok tvos "$(plist "$(modes audio)")" 2>&1); expect_status 1 $? "an Apple TV app with background audio" "$out"
out=$(background_modes_ok tvos "$(plist "$(modes fetch)")" 2>&1); expect_status 1 $? "an Apple TV app with background fetch" "$out"
out=$(background_modes_ok tvos "$(plist $'\t<key>UIBackgroundModes</key>\n\t<array/>\n')" 2>&1); expect_status 0 $? "an Apple TV app with an empty list" "$out"
# Other arrays' strings aren't background modes.
out=$(background_modes_ok tvos "$NONE" 2>&1); expect_not_in "_moonrise" "$out" "reads only UIBackgroundModes"

out=$(background_modes_ok ios "$IOS" 2>&1); expect_status 0 $? "an iOS app with audio, fetch and processing" "$out"
out=$(background_modes_ok ios "$NONE" 2>&1); expect_status 1 $? "an iOS app without background modes" "$out"
expect_in "doesn't declare background mode audio" "$out" "says which is missing"
out=$(background_modes_ok ios "$(plist "$(modes fetch processing)")" 2>&1); expect_status 1 $? "an iOS app without audio" "$out"
expect_in "doesn't declare background mode audio (it has: fetch processing)" "$out" "and what it has"

out=$(background_modes_ok tvos "" 2>&1); expect_status 1 $? "no plist (plutil failed)" "$out"
expect_in "no Info.plist to check" "$out" "says so"
out=$(background_modes_ok watchos "$NONE" 2>&1); expect_status 2 $? "an unknown platform" "$out"

# project.yml: iOS keeps the modes in the shared plist; the Apple TV build phase drops them.
YML="$REPO/clients/apple/project.yml"
grep -qE '^ *UIBackgroundModes: \[audio, fetch, processing\]' "$YML" && ok || fail "the shared Info.plist keeps iOS's background modes"
phase=$(awk '/^    postBuildScripts:/ { f = 1 } f && /^    [a-z]/ && !/postBuildScripts/ { exit } f' "$YML")
expect_in '- name: "Apple TV: no background modes"' "$phase" "the app has the Apple TV build phase"
expect_in '$(TARGET_BUILD_DIR)/$(INFOPLIST_PATH)' "$phase" "it runs after the built Info.plist exists (its input)"
expect_in 'case "$PLATFORM_NAME" in appletv*) ;; *) exit 0 ;; esac' "$phase" "it leaves every platform but the Apple TV alone"
expect_in 'for key in UIBackgroundModes BGTaskSchedulerPermittedIdentifiers' "$phase" "it drops the modes and the background task ids"
grep -qE '^ *"ENABLE_USER_SCRIPT_SANDBOXING\[sdk=appletv\*\]": NO' "$YML" && ok || fail "the Apple TV build lets the phase write the built plist (no script sandbox)"

done_testing
