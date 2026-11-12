#!/usr/bin/env bash
# MoonriseTests stopped running when the build Mac moved to another Xcode, because the test
# simulator came from fixed guesses. scripts/sim-new.sh picks the newest installed runtime of the OS
# and a device type it takes, prints only the UDID (whatever else simctl prints), and fails with
# simctl's message; scripts/test-apple.sh gives xcodebuild "platform=iOS Simulator,id=<that UDID>"
# and deletes the simulator after. xcrun, xcodebuild, rsync and ssh are stubs; nothing leaves here.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
C=$(fresh_clone)
mkdir -p "$SCRATCH/bin"
UDID=0A1B2C3D-4E5F-4A6B-8C7D-9E0F1A2B3C4D

# xcrun simctl stub. RUNTIMES / TYPES: what `list` prints; ACCEPT: the device type create takes
# (any if empty); CREATE_NOISE: extra lines create prints around the UDID. Calls go to xcrun-calls.
cat > "$SCRATCH/bin/xcrun" <<STUB
#!/bin/bash
echo "xcrun \$*" >> "$SCRATCH/xcrun-calls"
[ "\$1" = simctl ] || exit 0
case "\$2 \$3" in
  "list runtimes") printf '== Runtimes ==\n%b' "\$RUNTIMES" ;;
  "list devicetypes") printf '== Device Types ==\n%b' "\$TYPES" ;;
  "list devices")
    echo '== Devices ==' ; echo '-- iOS 26.3 --'
    [ -n "\${EXISTING:-}" ] && echo "    \$EXISTING"
    [ -f "$SCRATCH/made" ] && cat "$SCRATCH/made"
    exit 0 ;;
  create*)
    # NO_ID: create succeeds and prints nothing; the simulator is listed from then on.
    if [ -n "\${NO_ID:-}" ]; then echo "    \$3 (\$NO_ID) (Shutdown)" >> "$SCRATCH/made"; exit 0; fi
    if [ -n "\${ACCEPT:-}" ] && [ "\$4" != "\$ACCEPT" ]; then echo "An error was encountered processing the command (domain=com.apple.CoreSimulator.SimError, code=403): Incompatible device" >&2; exit 148; fi
    [ -n "\${CREATE_NOISE:-}" ] && echo "\$CREATE_NOISE"
    echo "$UDID"
    [ -n "\${CREATE_NOISE:-}" ] && echo "\$CREATE_NOISE"
    exit 0 ;;
esac
exit 0
STUB
chmod +x "$SCRATCH/bin/xcrun"
X26_RUNTIMES='iOS 26.3 (26.3.1 - 23D8133) - com.apple.CoreSimulator.SimRuntime.iOS-26-3\ntvOS 26.2 (26.2 - 23K51) - com.apple.CoreSimulator.SimRuntime.tvOS-26-2\n'
X26_TYPES='iPhone 17 Pro (com.apple.CoreSimulator.SimDeviceType.iPhone-17-Pro)\niPhone 17 (com.apple.CoreSimulator.SimDeviceType.iPhone-17)\niPhone 16 (com.apple.CoreSimulator.SimDeviceType.iPhone-16)\nApple TV 4K (3rd generation) (com.apple.CoreSimulator.SimDeviceType.Apple-TV-4K-3rd-generation-4K)\n'
# Another Xcode: several runtimes, not in version order, and device types without iPhone 15-17.
X27_RUNTIMES='iOS 27.1 (27.1 - 24B83) - com.apple.CoreSimulator.SimRuntime.iOS-27-1\niOS 26.10 (26.10 - 23X1) - com.apple.CoreSimulator.SimRuntime.iOS-26-10\niOS 26.3 (26.3.1 - 23D8133) - com.apple.CoreSimulator.SimRuntime.iOS-26-3\nwatchOS 27.0 (27.0 - 24R1) - com.apple.CoreSimulator.SimRuntime.watchOS-27-0\n'
X27_TYPES='iPhone 18 (com.apple.CoreSimulator.SimDeviceType.iPhone-18)\niPhone 18 Pro (com.apple.CoreSimulator.SimDeviceType.iPhone-18-Pro)\niPad Pro 13-inch (M6) (com.apple.CoreSimulator.SimDeviceType.iPad-Pro-13-inch-M6)\n'
simnew() { rm -f "$SCRATCH/xcrun-calls"; env PATH="$SCRATCH/bin:$PATH" bash "$C/scripts/sim-new.sh" "$@" 2>"$SCRATCH/err"; }
creates() { grep 'simctl create' "$SCRATCH/xcrun-calls" 2>/dev/null; }

out=$(RUNTIMES=$X26_RUNTIMES TYPES=$X26_TYPES simnew iOS t1); rc=$?
expect_status 0 $rc "Xcode 26: an iOS simulator" "$(cat "$SCRATCH/err")"
[ "$out" = "$UDID" ] && ok || fail "Xcode 26: printed '$out', wanted the UDID alone"
expect_in "simctl create t1 iPhone 17 com.apple.CoreSimulator.SimRuntime.iOS-26-3" "$(creates)" "Xcode 26: iPhone 17 on iOS 26.3"

out=$(RUNTIMES=$X27_RUNTIMES TYPES=$X27_TYPES ACCEPT='iPhone 18 Pro' simnew iOS t2); rc=$?
expect_status 0 $rc "Xcode 27: an iOS simulator" "$(cat "$SCRATCH/err")"
[ "$out" = "$UDID" ] && ok || fail "Xcode 27: printed '$out', wanted the UDID alone"
expect_in "simctl create t2 iPhone 18 Pro com.apple.CoreSimulator.SimRuntime.iOS-27-1" "$(creates)" "Xcode 27: the newest runtime, a listed iPhone"
expect_not_in "iPad" "$(creates)" "Xcode 27: only iPhones"

out=$(RUNTIMES=$X26_RUNTIMES TYPES=$X26_TYPES CREATE_NOISE='Warning: something about the runtime' simnew iOS t3); rc=$?
expect_status 0 $rc "create with extra output" "$(cat "$SCRATCH/err")"
[ "$out" = "$UDID" ] && ok || fail "create with extra output: printed '$out', wanted the UDID alone"

out=$(RUNTIMES=$X26_RUNTIMES TYPES=$X26_TYPES simnew tvOS t4); rc=$?
expect_status 0 $rc "a tvOS simulator" "$(cat "$SCRATCH/err")"
expect_in "simctl create t4 Apple TV 4K (3rd generation) com.apple.CoreSimulator.SimRuntime.tvOS-26-2" "$(creates)" "tvOS: Apple TV 4K on tvOS 26.2"

out=$(RUNTIMES='tvOS 26.2 (26.2 - 23K51) - com.apple.CoreSimulator.SimRuntime.tvOS-26-2\n' TYPES=$X26_TYPES simnew iOS t5); rc=$?
expect_status 1 $rc "no iOS runtime" "$out"
expect_in "no iOS simulator runtime installed" "$(cat "$SCRATCH/err")" "says there is no runtime"
[ -z "$out" ] && ok || fail "no iOS runtime: printed '$out' on stdout"

out=$(RUNTIMES=$X26_RUNTIMES TYPES=$X26_TYPES ACCEPT='iPhone 99' simnew iOS t6); rc=$?
expect_status 1 $rc "no device type works" "$out"
expect_in "Incompatible device" "$(cat "$SCRATCH/err")" "passes on simctl's message"

# Create exits 0 but prints no UDID: the simulator it made (found by name, and not one
# already there with that name) is deleted, and it fails without trying other device types.
NEW=11111111-2222-4333-8444-555555555555; OLD=99999999-8888-4777-8666-555555555555
rm -f "$SCRATCH/made"
out=$(RUNTIMES=$X26_RUNTIMES TYPES=$X26_TYPES NO_ID=$NEW EXISTING="t7 ($OLD) (Shutdown)" simnew iOS t7); rc=$?
expect_status 1 $rc "create printed no UDID" "$(cat "$SCRATCH/err")"
[ -z "$out" ] && ok || fail "no UDID: printed '$out' on stdout"
expect_in "simctl delete $NEW" "$(cat "$SCRATCH/xcrun-calls")" "the simulator it made is deleted"
expect_not_in "$OLD" "$(cat "$SCRATCH/xcrun-calls")" "an older simulator with the same name is left alone"
expect_in "printed no UDID" "$(cat "$SCRATCH/err")" "says create printed no UDID"
expect_in "deleted the simulator it made: t7 ($NEW)" "$(cat "$SCRATCH/err")" "says it deleted it"
[ "$(creates | wc -l)" = 1 ] && ok || fail "no UDID: $(creates | wc -l) creates, wanted 1"
rm -f "$SCRATCH/made"

# ---- scripts/test-apple.sh, with the Mac as a local shell in $SCRATCH/mac (its home folder).
mkdir -p "$SCRATCH/mac"
cat > "$SCRATCH/bin/ssh" <<STUB
#!/bin/bash
while [ \$# -gt 0 ]; do case "\$1" in -o) shift 2 ;; -*) shift ;; *) break ;; esac; done
shift   # the host
cd "$SCRATCH/mac" && HOME="$SCRATCH/mac" exec bash -c "\$*"
STUB
cat > "$SCRATCH/bin/rsync" <<STUB
#!/bin/bash
src=\${@: -2:1}; mkdir -p "$SCRATCH/mac/vp-tests" && cp -R "\$src." "$SCRATCH/mac/vp-tests/"
STUB
cat > "$SCRATCH/bin/xcodebuild" <<STUB
#!/bin/bash
echo "xcodebuild \$*" >> "$SCRATCH/xcodebuild-calls"
[ "\$1" = -version ] && { printf 'Xcode 27.1\nBuild version 27A1\n'; exit 0; }
echo '✔ Test run with 3 tests in 1 suite passed after 0.1 seconds.'; echo '** TEST SUCCEEDED **'
STUB
cat > "$SCRATCH/bin/xcodegen" <<'STUB'
#!/bin/sh
[ "$1" = --version ] && echo "Version: 0" ; exit 0
STUB
chmod +x "$SCRATCH/bin/"*
rm -f "$SCRATCH/xcrun-calls"
out=$(env PATH="$SCRATCH/bin:$PATH" MAC=tester@mac.invalid MAC_TEST_DIR=vp-tests ONLY=SomeTests \
  RUNTIMES="$X27_RUNTIMES" TYPES="$X27_TYPES" bash "$C/scripts/test-apple.sh" 2>&1); rc=$?
expect_status 0 $rc "test-apple.sh on Xcode 27" "$out"
expect_in "Apple tests passed" "$out" "test-apple.sh reports a pass"
expect_in "warning: Xcode 27.1 here" "$out" "another Xcode only warns"
expect_in "-destination platform=iOS Simulator,id=$UDID " "$(cat "$SCRATCH/xcodebuild-calls")" "xcodebuild gets the simulator by platform and UDID"
expect_in "simctl delete $UDID" "$(cat "$SCRATCH/xcrun-calls")" "the test simulator is deleted after"
done_testing
