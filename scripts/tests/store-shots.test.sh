#!/usr/bin/env bash
# scripts/store-shots.sh, offline: it runs on the release Mac only, as scripts/release-apple.sh does.
# - MAC and FASTLANE_MAC (which may name someone's own Mac) are never used, in mac.env or in the
#   environment: without RELEASE_MAC it stops before any ssh, rsync or scp.
# - a Mac in PERSONAL_MACS is refused, and so is RELEASE_MAC=local.
# - with RELEASE_MAC, everything goes there, the build (scripts/build.sh) included, into the
#   script's own folder, ~/moonrise-store-shots (SHOTS_DIR), even when mac.env sets MAC and MAC_DIR.
# - the environment's RELEASE_MAC wins over mac.env's.
# - the app is STORE_BUNDLE_ID, never the signing file's MOONRISE_BUNDLE_ID (the id device installs
#   use): the build gets it and the simulator launches it; without it the shots are refused before
#   reaching a Mac, and so is DEV=1.
# The build Mac is a stub `ssh`/`rsync`/`scp` that logs each call.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
C=$(fresh_clone)
printf 'MOONRISE_BUNDLE_ID = org.example.device\nMOONRISE_DEVELOPMENT_TEAM = ABCDE12345\n' > "$C/clients/apple/Config/Signing.local.xcconfig"
mkdir -p "$SCRATCH/bin"
LOG="$SCRATCH/calls.log"; export LOG
printf '#!/bin/bash\necho "rsync $*" >> "$LOG"\n' > "$SCRATCH/bin/rsync"
# scp: a minimal JPEG (SOI, EOI) where the screenshot goes.
cat > "$SCRATCH/bin/scp" <<'STUB'
#!/bin/bash
echo "scp $*" >> "$LOG"
for a; do to=$a; done
printf '\xff\xd8\xff\xd9' > "$to"
STUB
# ssh: a simulator's UDID when asked for one; the simulator lock (sim_lock) taken at once and held
# until stdin closes.
cat > "$SCRATCH/bin/ssh" <<'STUB'
#!/bin/bash
case "$*" in *" sim-lock "*) echo "ssh $(printf '%s\n' "$@" | grep -m1 @) sim-lock" >> "$LOG"; echo locked; cat >/dev/null; exit 0 ;; esac
echo "ssh $*" >> "$LOG"
case "$*" in
  *"simctl list devices available"*) echo 11111111-2222-3333-4444-555555555555 ;;
esac
exit 0
STUB
chmod +x "$SCRATCH/bin/"*
run() {
  rm -f "$LOG"; touch "$LOG"
  env -u MAC -u FASTLANE_MAC -u TSKIT_MAC -u PERSONAL_MACS -u MAC_DIR -u MOONRISE_RELEASE_DIR -u MOONRISE_STORE_BUILD \
    RELEASE_MAC="${RELEASE_MAC-}" STORE_BUNDLE_ID="${STORE_BUNDLE_ID-org.example.store}" PATH="$SCRATCH/bin:$PATH" bash "$C/scripts/store-shots.sh" "$@" 2>&1
}
reached() { [ -s "$LOG" ]; }

# mac.env as a personal one is set up: MAC, FASTLANE_MAC and TSKIT_MAC at someone's own Mac.
ME=owner@192.168.1.85
printf 'MAC=%s\nFASTLANE_MAC=$MAC\nTSKIT_MAC=$MAC\nMAC_DIR=video-player\n' "$ME" > "$C/scripts/mac.env"

out=$(RELEASE_MAC= run phone); rc=$?
expect_status 1 $rc "no RELEASE_MAC, with MAC and FASTLANE_MAC in mac.env" "$out"
expect_in "no release Mac: set RELEASE_MAC" "$out" "asks for RELEASE_MAC"
reached && fail "without RELEASE_MAC it reached a Mac" "$(cat "$LOG")" || ok
out=$(RELEASE_MAC= MAC=$ME FASTLANE_MAC=$ME run phone); rc=$?
expect_status 1 $rc "no RELEASE_MAC, with MAC and FASTLANE_MAC in the environment" "$out"
reached && fail "MAC / FASTLANE_MAC from the environment reached a Mac" "$(cat "$LOG")" || ok
out=$(RELEASE_MAC= run --delete phone); rc=$?
expect_status 1 $rc "--delete without RELEASE_MAC" "$out"
reached && fail "--delete without RELEASE_MAC reached a Mac" "$(cat "$LOG")" || ok

out=$(RELEASE_MAC=builder@192.168.1.65 run phone); rc=$?
expect_status 0 $rc "phone shots with RELEASE_MAC" "$out"
calls=$(cat "$LOG")
expect_in "ssh builder@192.168.1.65 " "$calls" "the shots run on RELEASE_MAC"
expect_in "builder@192.168.1.65:~/moonrise-store-shots/" "$calls" "build.sh copies to RELEASE_MAC, into the shots' own folder"
expect_in "xcodebuild build" "$calls" "the build ran"
expect_in "scp -q builder@192.168.1.65:/tmp/moonrise-store-shot.jpg" "$calls" "the shots come from RELEASE_MAC"
expect_not_in "$ME" "$calls" "never mac.env's MAC"
expect_not_in "~/video-player" "$calls" "never mac.env's MAC_DIR"
[ -f "$C/clients/apple/fastlane/screenshots/ios/en-US/1-phone-home.jpg" ] && ok || fail "no 1-phone-home.jpg" "$out"
# Every call went to RELEASE_MAC: each ssh/scp/rsync call (a log line starting with its name) names it.
others=$(grep -E '^(ssh|scp|rsync) ' "$LOG" | grep -v 'builder@192.168.1.65')
[ -z "$others" ] && ok || fail "calls not to RELEASE_MAC" "$others"
# The App Store app: built and launched as STORE_BUNDLE_ID, never the signing file's device id.
expect_in "MOONRISE_BUNDLE_ID=org.example.store" "$(grep -A3 'xcodebuild build' "$LOG")" "the build is STORE_BUNDLE_ID (after the xcconfig)"
expect_in "xcrun simctl launch 11111111-2222-3333-4444-555555555555 org.example.store" "$calls" "launches STORE_BUNDLE_ID"
expect_not_in "org.example.device" "$calls" "never the signing file's device id"
expect_not_in "org.example.device" "$out" "never the signing file's device id (output)"
out=$(RELEASE_MAC=builder@192.168.1.65 STORE_BUNDLE_ID= run phone); rc=$?
expect_status 1 $rc "shots without STORE_BUNDLE_ID" "$out"
expect_in "set STORE_BUNDLE_ID in scripts/mac.env" "$out" "says what to set"
reached && fail "without STORE_BUNDLE_ID it reached a Mac" "$(cat "$LOG")" || ok
out=$(RELEASE_MAC=builder@192.168.1.65 DEV=1 run phone); rc=$?
expect_status 2 $rc "shots with DEV=1" "$out"
reached && fail "DEV=1 reached a Mac" "$(cat "$LOG")" || ok

printf 'RELEASE_MAC=builder@192.168.1.65\n' >> "$C/scripts/mac.env"
out=$(RELEASE_MAC= run --delete tv); rc=$?
expect_status 0 $rc "RELEASE_MAC from mac.env" "$out"
expect_in "ssh builder@192.168.1.65 " "$(cat "$LOG")" "uses mac.env's RELEASE_MAC"
out=$(RELEASE_MAC=other@192.168.1.66 run --delete tv)
expect_in "ssh other@192.168.1.66 " "$(cat "$LOG")" "the environment's RELEASE_MAC wins"
expect_not_in "builder@192.168.1.65" "$(cat "$LOG")" "over mac.env's"

printf 'PERSONAL_MACS="192.168.1.85 me@studio.example"\n' >> "$C/scripts/mac.env"
for m in $ME someone@192.168.1.85 x@STUDIO.example; do
  out=$(RELEASE_MAC=$m run phone); rc=$?
  expect_status 1 $rc "RELEASE_MAC=$m, a Mac in PERSONAL_MACS" "$out"
  expect_in "is listed in PERSONAL_MACS" "$out" "says why ($m)"
  reached && fail "a personal Mac ($m) was reached" "$(cat "$LOG")" || ok
done
out=$(RELEASE_MAC=local run phone); rc=$?
expect_status 1 $rc "RELEASE_MAC=local" "$out"
reached && fail "RELEASE_MAC=local ran something" "$(cat "$LOG")" || ok
# On a Mac (local mode by default) it still goes to RELEASE_MAC, over ssh.
out=$(RELEASE_MAC=builder@192.168.1.65 MOONRISE_UNAME=Darwin run --delete phone); rc=$?
expect_status 0 $rc "on a Mac, with RELEASE_MAC" "$out"
expect_in "ssh builder@192.168.1.65 " "$(cat "$LOG")" "on a Mac, over ssh to RELEASE_MAC"
rm -f "$C/scripts/mac.env"
done_testing
