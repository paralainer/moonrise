#!/usr/bin/env bash
# scripts/ota.sh, offline: the ad-hoc IPA is built on the release Mac only, as the App Store builds are.
# - MAC and FASTLANE_MAC (which may name someone's own Mac) are never used, in mac.env or in the
#   environment: without RELEASE_MAC it stops before any ssh, rsync or scp.
# - a Mac in PERSONAL_MACS is refused, and so is RELEASE_MAC=local.
# - with RELEASE_MAC, everything goes there, into the script's own folder, ~/moonrise-ota-build
#   (OTA_MAC_DIR), even when mac.env sets MAC and MAC_DIR; the environment's RELEASE_MAC wins.
# And the app's tests (test-apple.sh) skip a MAC listed in PERSONAL_MACS, before any ssh.
# The build Mac is a stub `ssh`/`rsync`/`scp` that logs each call; `docker` (the OTA container) fails.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
C=$(fresh_clone)
mkdir -p "$SCRATCH/bin"
printf 'MOONRISE_BUNDLE_ID = org.example.store\nMOONRISE_DEVELOPMENT_TEAM = ABCDE12345\n' > "$C/clients/apple/Config/Signing.local.xcconfig"
LOG="$SCRATCH/calls.log"; export LOG
printf '#!/bin/bash\necho "rsync $*" >> "$LOG"\n' > "$SCRATCH/bin/rsync"
printf '#!/bin/bash\necho "ssh $*" >> "$LOG"\n' > "$SCRATCH/bin/ssh"
printf '#!/bin/bash\nexit 1\n' > "$SCRATCH/bin/docker"
# scp: an empty IPA where it goes.
cat > "$SCRATCH/bin/scp" <<'STUB'
#!/bin/bash
echo "scp $*" >> "$LOG"
for a; do to=$a; done
: > "$to"
STUB
chmod +x "$SCRATCH/bin/"*
run() {
  rm -f "$LOG"; touch "$LOG"; rm -rf "$SCRATCH/ota"
  env -u MAC -u FASTLANE_MAC -u TSKIT_MAC -u PERSONAL_MACS -u MAC_DIR -u MOONRISE_RELEASE_DIR -u DEV \
    RELEASE_MAC="${RELEASE_MAC-}" OTA_MAC_DIR="${OTA_MAC_DIR-}" OTA_DIR="$SCRATCH/ota" PATH="$SCRATCH/bin:$PATH" bash "$C/scripts/ota.sh" "$@" 2>&1
}
reached() { [ -s "$LOG" ]; }

# mac.env as a personal one is set up: MAC and FASTLANE_MAC at someone's own Mac.
ME=owner@192.168.1.85
printf 'MAC=%s\nFASTLANE_MAC=$MAC\nTSKIT_MAC=$MAC\nMAC_DIR=video-player\n' "$ME" > "$C/scripts/mac.env"

out=$(RELEASE_MAC= run); rc=$?
expect_status 1 $rc "no RELEASE_MAC, with MAC and FASTLANE_MAC in mac.env" "$out"
expect_in "no release Mac: set RELEASE_MAC" "$out" "asks for RELEASE_MAC"
reached && fail "without RELEASE_MAC it reached a Mac" "$(cat "$LOG")" || ok
out=$(RELEASE_MAC= MAC=$ME FASTLANE_MAC=$ME run); rc=$?
expect_status 1 $rc "no RELEASE_MAC, with MAC and FASTLANE_MAC in the environment" "$out"
reached && fail "MAC / FASTLANE_MAC from the environment reached a Mac" "$(cat "$LOG")" || ok

out=$(RELEASE_MAC=builder@192.168.1.65 run); rc=$?
expect_status 0 $rc "OTA build with RELEASE_MAC" "$out"
calls=$(cat "$LOG")
expect_in "builder@192.168.1.65:~/moonrise-ota-build/" "$calls" "copies the tree to RELEASE_MAC, into its own folder"
expect_in "ssh builder@192.168.1.65 " "$calls" "builds on RELEASE_MAC"
expect_in "xcodebuild archive" "$calls" "the archive ran"
expect_in "scp -q builder@192.168.1.65:~/moonrise-ota-build/build-ota/export/Moonrise.ipa" "$calls" "the IPA comes from RELEASE_MAC"
expect_not_in "$ME" "$calls" "never mac.env's MAC"
expect_not_in "~/video-player" "$calls" "never mac.env's MAC_DIR"
[ -f "$SCRATCH/ota/manifest.plist" ] && ok || fail "no manifest.plist" "$out"
others=$(grep -E '^(ssh|scp|rsync) ' "$LOG" | grep -v 'builder@192.168.1.65')
[ -z "$others" ] && ok || fail "calls not to RELEASE_MAC" "$others"
out=$(RELEASE_MAC=builder@192.168.1.65 OTA_MAC_DIR=elsewhere run); rc=$?
expect_in "builder@192.168.1.65:~/elsewhere/" "$(cat "$LOG")" "OTA_MAC_DIR names the folder"

printf 'RELEASE_MAC=builder@192.168.1.65\n' >> "$C/scripts/mac.env"
out=$(RELEASE_MAC= run); rc=$?
expect_status 0 $rc "RELEASE_MAC from mac.env" "$out"
expect_in "ssh builder@192.168.1.65 " "$(cat "$LOG")" "uses mac.env's RELEASE_MAC"
out=$(RELEASE_MAC=other@192.168.1.66 run)
expect_in "ssh other@192.168.1.66 " "$(cat "$LOG")" "the environment's RELEASE_MAC wins"
expect_not_in "builder@192.168.1.65" "$(cat "$LOG")" "over mac.env's"

printf 'PERSONAL_MACS="192.168.1.85 me@studio.example"\n' >> "$C/scripts/mac.env"
for m in $ME someone@192.168.1.85 x@STUDIO.example; do
  out=$(RELEASE_MAC=$m run); rc=$?
  expect_status 1 $rc "RELEASE_MAC=$m, a Mac in PERSONAL_MACS" "$out"
  expect_in "is listed in PERSONAL_MACS" "$out" "says why ($m)"
  reached && fail "a personal Mac ($m) was reached" "$(cat "$LOG")" || ok
done
out=$(RELEASE_MAC=local run); rc=$?
expect_status 1 $rc "RELEASE_MAC=local" "$out"
reached && fail "RELEASE_MAC=local ran something" "$(cat "$LOG")" || ok
out=$(RELEASE_MAC=builder@192.168.1.65 MOONRISE_UNAME=Darwin run); rc=$?
expect_status 0 $rc "on a Mac, with RELEASE_MAC" "$out"
expect_in "ssh builder@192.168.1.65 " "$(cat "$LOG")" "on a Mac, over ssh to RELEASE_MAC"

# The app's tests: mac.env's MAC is in PERSONAL_MACS, so test-apple.sh skips (exit 3) without ssh.
rm -f "$LOG"; touch "$LOG"
out=$(env -u MAC -u PERSONAL_MACS PATH="$SCRATCH/bin:$PATH" bash "$C/scripts/test-apple.sh" 2>&1); rc=$?
expect_status 3 $rc "test-apple.sh with a personal MAC" "$out"
expect_in "is listed in PERSONAL_MACS" "$out" "says why"
reached && fail "test-apple.sh reached a personal Mac" "$(cat "$LOG")" || ok
out=$(env -u PERSONAL_MACS MAC=someone@STUDIO.example PATH="$SCRATCH/bin:$PATH" bash "$C/scripts/test-apple.sh" 2>&1); rc=$?
expect_status 3 $rc "test-apple.sh with a personal MAC from the environment" "$out"
reached && fail "test-apple.sh reached a personal Mac" "$(cat "$LOG")" || ok
rm -f "$C/scripts/mac.env"
done_testing
