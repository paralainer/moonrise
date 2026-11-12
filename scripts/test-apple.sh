#!/usr/bin/env bash
# Run the Apple apps' unit tests (MoonriseTests, Swift Testing) on the build Mac, on a simulator
# made for this run and deleted afterwards, then the macOS-only keep-awake check
# (scripts/tests/keep-awake, built natively), an unsigned compile of the macOS app, and the Apple
# TV UI tests (MoonriseTVUITests, on a tvOS simulator also made for the run; these three not with
# ONLY). Offline: no server, no sign-in.
#
#   scripts/test-apple.sh                  # MAC from scripts/mac.env, or MAC=user@host
#   ONLY=SegmentRulesTests scripts/test-apple.sh   # one suite (xcodebuild -only-testing)
#   ONLY=TVHomeFocusUITests scripts/test-apple.sh  # a name ending in UITests: that Apple TV UI suite
#
# The tree is copied to ~/$MAC_TEST_DIR on the Mac (default moonrise-tests), apart from the build
# folder scripts/build.sh uses, so a test run and a build don't collide. Exits 0 when every test
# passes, 1 when one fails or the build breaks, 3 when the Mac can't be reached (scripts/test-all.sh
# reports that as skipped).
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/common.sh"
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAC_TEST_DIR=${MAC_TEST_DIR:-moonrise-tests}
if [ -z "$MAC" ]; then
  echo "no build Mac (MAC= or scripts/mac.env): Apple tests not run" >&2
  exit 3
fi
if ! mac_reachable; then
  echo "can't reach the build Mac $MAC over SSH ($MAC_SSH_TRIES attempts): Apple tests not run" >&2
  exit 3
fi
ONLY_ARG=
TV_ONLY_ARG=
case "${ONLY:-}" in
  "") ;;
  *UITests) TV_ONLY_ARG="-only-testing:MoonriseTVUITests/$ONLY" ;;
  *) ONLY_ARG="-only-testing:MoonriseTests/$ONLY" ;;
esac

# The Apple TV UI tests, on a tvOS simulator made for them and deleted afterwards (not retried, as
# the unit tests' ssh below).
run_tv_ui_tests() {
  ssh "${MAC_SSH_OPTS[@]}" "$MAC" "$MAC_PATH
set -u
cd ~/$MAC_TEST_DIR || exit 1
xg=\$(scripts/xcodegen.sh 2>&1 >/dev/null) || { echo \"\$xg\" >&2; echo 'xcodegen failed' >&2; exit 1; }
sim=\$(scripts/sim-new.sh tvOS \"moonrise-tv-tests-\$\$\") || { echo 'could not create an Apple TV test simulator' >&2; exit 1; }
trap 'xcrun simctl shutdown \$sim >/dev/null 2>&1; xcrun simctl delete \$sim >/dev/null 2>&1' EXIT
xcodebuild test -project $APPLE_DIR/Moonrise.xcodeproj -scheme MoonriseTVUITests -configuration Debug \
  -destination \"platform=tvOS Simulator,id=\$sim\" -derivedDataPath build-tests-tv $TV_ONLY_ARG \
  CODE_SIGN_IDENTITY=- CODE_SIGN_STYLE=Manual DEVELOPMENT_TEAM= PROVISIONING_PROFILE_SPECIFIER= \
  > build-tests-tv.log 2>&1; rc=\$?
grep -E ': error: |Test Case .* failed|Executed [0-9]+ test|\\*\\* TEST' build-tests-tv.log | head -40
exit \$rc"
}

echo "==> Apple unit tests on $MAC (~/$MAC_TEST_DIR)"
rsync_retry -a --delete --exclude .git --exclude /private --exclude "server/" --exclude /clients/android \
  --exclude "build-*" --exclude Moonrise.xcodeproj --exclude "*.log" --exclude Vendor --exclude /art --exclude /assets \
  "$DIR/" "$MAC:~/$MAC_TEST_DIR/"

# Only an Apple TV UI suite was asked for: no unit tests.
if [ -n "$TV_ONLY_ARG" ]; then
  if ! mac_reachable; then
    echo "can't reach the build Mac $MAC over SSH ($MAC_SSH_TRIES attempts): Apple tests not run" >&2
    exit 3
  fi
  if run_tv_ui_tests; then echo "==> Apple tests passed"; exit 0; fi
  echo "==> Apple tests FAILED (full log: ~/$MAC_TEST_DIR/build-tests-tv.log on $MAC)" >&2
  exit 1
fi

# The simulator: a new one for this run (never one somebody else uses), deleted on the way out.
# Unsigned identity as for scripts/build.sh sim: ad-hoc signed, no team needed. This ssh is not
# retried (a dropped connection may leave the tests running on the Mac); the reachability check
# just before it is.
if ! mac_reachable; then
  echo "can't reach the build Mac $MAC over SSH ($MAC_SSH_TRIES attempts): Apple tests not run" >&2
  exit 3
fi
ssh "${MAC_SSH_OPTS[@]}" "$MAC" "$MAC_PATH
set -u
cd ~/$MAC_TEST_DIR || exit 1
xg=\$(scripts/xcodegen.sh 2>&1 >/dev/null) || { echo \"\$xg\" >&2; echo 'xcodegen failed' >&2; exit 1; }
printf '%s\n' \"\$xg\" | grep -E '^(warning|error):' >&2 || true
sim=\$(scripts/sim-new.sh iOS \"moonrise-tests-\$\$\") || { echo 'could not create a test simulator' >&2; exit 1; }
trap 'xcrun simctl shutdown \$sim >/dev/null 2>&1; xcrun simctl delete \$sim >/dev/null 2>&1' EXIT
xcodebuild test -project $APPLE_DIR/Moonrise.xcodeproj -scheme MoonriseTests -configuration Debug \
  -destination \"platform=iOS Simulator,id=\$sim\" -derivedDataPath build-tests $ONLY_ARG \
  CODE_SIGN_IDENTITY=- CODE_SIGN_STYLE=Manual DEVELOPMENT_TEAM= PROVISIONING_PROFILE_SPECIFIER= \
  > build-tests.log 2>&1; rc=\$?
grep -E '(error|warning: unre).*:' build-tests.log | grep -v '^ld: warning' | sort -u | head -40
# A destination xcodebuild won't take: show what it would take.
if grep -q 'destination' build-tests.log && [ \$rc -ne 0 ]; then
  echo \"xcodebuild -showdestinations (simulator \$sim):\" >&2
  xcodebuild -showdestinations -project $APPLE_DIR/Moonrise.xcodeproj -scheme MoonriseTests 2>&1 | grep -E '\\{ platform' | head -20 >&2
fi
grep -E '✘|recorded an issue|Expectation failed|Test run with|\\*\\* TEST' build-tests.log | head -80
# The Mac's real keep-awake assertion (macOS only, so not in MoonriseTests): built and run natively.
if [ -z \"$ONLY_ARG\" ]; then
  mkdir -p build-tests
  if xcrun --sdk macosx swiftc -o build-tests/keep-awake $APPLE_DIR/Moonrise/Player/KeepAwake.swift scripts/tests/keep-awake/main.swift \
       > build-tests/keep-awake.log 2>&1; then
    build-tests/keep-awake || rc=1
  else
    tail -20 build-tests/keep-awake.log; rc=1
  fi
  # The macOS app compiles: MoonriseTests run on iOS, so a Mac-only break showed nowhere.
  # Unsigned compile, as scripts/build.sh macos.
  if xcodebuild build -project $APPLE_DIR/Moonrise.xcodeproj -scheme Moonrise -configuration Debug \\
       -destination 'generic/platform=macOS' -derivedDataPath build-tests-macos CODE_SIGNING_ALLOWED=NO \\
       > build-tests-macos.log 2>&1; then
    echo 'macOS app: compiles'
  else
    grep -E 'error.*:' build-tests-macos.log | sort -u | head -40
    echo 'macOS app: BUILD FAILED (build-tests-macos.log)'; rc=1
  fi
fi
exit \$rc" && rc=0 || rc=$?
if [ $rc -ne 0 ]; then echo "==> Apple tests FAILED (full log: ~/$MAC_TEST_DIR/build-tests.log on $MAC)" >&2; exit 1; fi

# The whole run: the Apple TV UI tests too.
if [ -z "${ONLY:-}" ]; then
  if ! mac_reachable; then
    echo "can't reach the build Mac $MAC over SSH ($MAC_SSH_TRIES attempts): Apple TV UI tests not run" >&2
    exit 3
  fi
  if ! run_tv_ui_tests; then
    echo "==> Apple tests FAILED (full log: ~/$MAC_TEST_DIR/build-tests-tv.log on $MAC)" >&2
    exit 1
  fi
fi
echo "==> Apple tests passed"
