#!/usr/bin/env bash
# scripts/release-apple.sh, offline:
# - check-metadata: the App Store listing in clients/apple/fastlane/metadata is within App Store
#   Connect's limits (name and subtitle 30 characters, keywords 100, promotional text 170,
#   description 4000, review notes 4000; https URLs; keywords without spaces around commas), and a
#   listing over a limit is refused, naming the file. Characters, not bytes.
# - next-build: one past the latest TestFlight build fastlane reports (through its colour codes);
#   BUILD_NUMBER wins.
# - the store version is MARKETING_VERSION in project.yml.
# - submit goes nowhere without --yes.
# - the description doesn't sell basics (Continue Watching, resume, Top Shelf).
# - the Mac is RELEASE_MAC only: MAC and FASTLANE_MAC (which may name someone's own Mac) are never
#   used, without RELEASE_MAC it stops before reaching any Mac, a Mac in PERSONAL_MACS is refused,
#   and the environment's RELEASE_MAC wins over mac.env's. build-tailscalekit.sh, which a store
#   build calls with TSKIT_MAC, keeps that Mac over mac.env's.
# The build Mac is a stub `ssh` that answers as fastlane would.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
C=$(fresh_clone)
mkdir -p "$C/clients/apple/fastlane" "$SCRATCH/bin"
cp -r "$REPO/clients/apple/fastlane/metadata" "$REPO/clients/apple/fastlane/review_notes.txt" "$C/clients/apple/fastlane/"
cp "$REPO/clients/apple/project.yml" "$C/clients/apple/"
printf 'MOONRISE_BUNDLE_ID = org.example.store\nMOONRISE_DEVELOPMENT_TEAM = ABCDE12345\n' > "$C/clients/apple/Config/Signing.local.xcconfig"
LOG="$SCRATCH/calls.log"; export LOG
printf '#!/bin/bash\necho "rsync $*" >> "$LOG"\n' > "$SCRATCH/bin/rsync"
cat > "$SCRATCH/bin/ssh" <<'STUB'
#!/bin/bash
echo "ssh $*" >> "$LOG"
case "$*" in
  *"fastlane next_build"*) printf '\033[32m[12:00:00]: Latest upload\033[0m\nMOONRISE_LATEST_BUILD=41\n' ;;
esac
exit 0
STUB
chmod +x "$SCRATCH/bin/"*
run() {
  rm -f "$LOG"; touch "$LOG"
  env -u MAC -u FASTLANE_MAC -u TSKIT_MAC -u PERSONAL_MACS RELEASE_MAC="${RELEASE_MAC-builder@example.invalid}" \
    PATH="$SCRATCH/bin:$PATH" ASC_ENV='~/asc.env' bash "$C/scripts/release-apple.sh" "$@" 2>&1
}

out=$(run check-metadata); rc=$?
expect_status 0 $rc "the repository's listing" "$out"
expect_in "within App Store limits" "$out" "says it fits"

M="$C/clients/apple/fastlane/metadata"
cp -r "$M" "$SCRATCH/metadata.orig"
restore() { rm -rf "$M"; cp -r "$SCRATCH/metadata.orig" "$M"; }

printf 'Moonrise Player for Home Media Libraries\n' > "$M/en-US/name.txt"
out=$(run check-metadata); rc=$?
expect_status 1 $rc "a 40-character name" "$out"
expect_in "too long: en-US/name.txt has 40 characters (App Store limit 30)" "$out" "names the file and the limit"
restore

# 30 characters with multi-byte ones (60 bytes) fit.
printf 'ÉÉÉÉÉÉÉÉÉÉÉÉÉÉÉÉÉÉÉÉÉÉÉÉÉÉÉÉÉÉ\n' > "$M/en-US/subtitle.txt"
out=$(run check-metadata); rc=$?
expect_status 0 $rc "a 30-character subtitle in two-byte characters" "$out"
restore

printf '%0101d\n' 0 > "$M/en-US/keywords.txt"
out=$(run check-metadata); rc=$?
expect_status 1 $rc "101 characters of keywords" "$out"
expect_in "keywords.txt has 101 characters" "$out" "says how long"
printf 'media server, NAS\n' > "$M/en-US/keywords.txt"
out=$(run check-metadata); rc=$?
expect_status 1 $rc "keywords with a space after a comma" "$out"
expect_in "no spaces around the commas" "$out" "says why"
restore

printf '%0171d\n' 0 > "$M/en-US/promotional_text.txt"
out=$(run check-metadata); rc=$?
expect_status 1 $rc "171 characters of promotional text" "$out"
restore

printf 'http://example.com/\n' > "$M/en-US/support_url.txt"
out=$(run check-metadata); rc=$?
expect_status 1 $rc "a plain http support URL" "$out"
expect_in "not an https URL: en-US/support_url.txt" "$out" "names it"
restore

mv "$C/clients/apple/fastlane/review_notes.txt" "$SCRATCH/notes"
out=$(run check-metadata); rc=$?
expect_status 1 $rc "no review notes" "$out"
mv "$SCRATCH/notes" "$C/clients/apple/fastlane/review_notes.txt"

out=$(run next-build ios); rc=$?
expect_status 0 $rc "next-build ios" "$out"
[ "$(tail -1 <<<"$out")" = 42 ] && ok || fail "next-build after build 41: $(tail -1 <<<"$out"), wanted 42" "$out"
expect_in "fastlane next_build platform:ios" "$(cat "$LOG")" "asks fastlane for the platform's latest"
expect_in "MOONRISE_BUNDLE_ID=org.example.store MOONRISE_VERSION=0.1.0" "$(cat "$LOG")" "passes the bundle id and project.yml's version"
out=$(BUILD_NUMBER=7 run next-build tvos); rc=$?
[ "$(tail -1 <<<"$out")" = 7 ] && ok || fail "BUILD_NUMBER=7: $(tail -1 <<<"$out")" "$out"
grep -q 'fastlane next_build' "$LOG" && fail "BUILD_NUMBER still asked fastlane" "$(cat "$LOG")" || ok
out=$(MOONRISE_VERSION=1.2.3 run next-build ios)
expect_in "MOONRISE_VERSION=1.2.3" "$(cat "$LOG")" "MOONRISE_VERSION overrides the version"
out=$(run next-build watchos); rc=$?
expect_status 2 $rc "an unknown platform" "$out"

out=$(run submit ios); rc=$?
expect_status 2 $rc "submit without --yes" "$out"
expect_in "the owner's decision" "$out" "says why"
[ -s "$LOG" ] && fail "submit without --yes reached the Mac" "$(cat "$LOG")" || ok
# The listing names differentiators only, not what every player has.
D=$(cat "$REPO/clients/apple/fastlane/metadata/en-US/description.txt")
for basic in "Continue Watching" "PRESS RESUME" "Top Shelf"; do
  expect_not_in "$basic" "$D" "the description's differentiators (a basic: $basic)"
done

# The release Mac. mac.env as a personal one is set up: MAC and FASTLANE_MAC at someone's own Mac.
ME=owner@192.168.1.85
printf 'MAC=%s\nFASTLANE_MAC=$MAC\nTSKIT_MAC=$MAC\n' "$ME" > "$C/scripts/mac.env"
out=$(RELEASE_MAC= run status); rc=$?
expect_status 1 $rc "no RELEASE_MAC, with MAC and FASTLANE_MAC in mac.env" "$out"
expect_in "no release Mac: set RELEASE_MAC" "$out" "asks for RELEASE_MAC"
[ -s "$LOG" ] && fail "without RELEASE_MAC it reached a Mac" "$(cat "$LOG")" || ok
out=$(RELEASE_MAC= MAC=$ME FASTLANE_MAC=$ME run status); rc=$?
expect_status 1 $rc "no RELEASE_MAC, with MAC and FASTLANE_MAC in the environment" "$out"
[ -s "$LOG" ] && fail "MAC / FASTLANE_MAC from the environment reached a Mac" "$(cat "$LOG")" || ok

out=$(RELEASE_MAC=builder@192.168.1.65 run status); rc=$?
expect_status 0 $rc "status with RELEASE_MAC" "$out"
expect_in "builder@192.168.1.65" "$(cat "$LOG")" "uses RELEASE_MAC"
expect_not_in "$ME" "$(cat "$LOG")" "never mac.env's MAC"

printf 'RELEASE_MAC=builder@192.168.1.65\n' >> "$C/scripts/mac.env"
out=$(RELEASE_MAC= run status); rc=$?
expect_status 0 $rc "RELEASE_MAC from mac.env" "$out"
expect_in "builder@192.168.1.65" "$(cat "$LOG")" "uses mac.env's RELEASE_MAC"
out=$(RELEASE_MAC=other@192.168.1.66 run status)
expect_in "other@192.168.1.66" "$(cat "$LOG")" "the environment's RELEASE_MAC wins"
expect_not_in "builder@192.168.1.65" "$(cat "$LOG")" "over mac.env's"

printf 'PERSONAL_MACS="192.168.1.85 me@studio.example"\n' >> "$C/scripts/mac.env"
for m in $ME someone@192.168.1.85 x@STUDIO.example; do
  out=$(RELEASE_MAC=$m run status); rc=$?
  expect_status 1 $rc "RELEASE_MAC=$m, a Mac in PERSONAL_MACS" "$out"
  expect_in "is listed in PERSONAL_MACS" "$out" "says why ($m)"
  [ -s "$LOG" ] && fail "a personal Mac ($m) was reached" "$(cat "$LOG")" || ok
done
out=$(RELEASE_MAC=builder@192.168.1.65 run status); rc=$?
expect_status 0 $rc "a Mac not in PERSONAL_MACS" "$out"
out=$(RELEASE_MAC=local run status); rc=$?
expect_status 1 $rc "RELEASE_MAC=local" "$out"
[ -s "$LOG" ] && fail "RELEASE_MAC=local ran something" "$(cat "$LOG")" || ok
out=$(RELEASE_MAC= run check-metadata); rc=$?
expect_status 0 $rc "check-metadata needs no Mac" "$out"

# A store build's TailscaleKit step: the Mac and folder release-apple.sh passes, not mac.env's.
printf 'MAC_DIR=video-player\n' >> "$C/scripts/mac.env"
rm -f "$LOG"; touch "$LOG"
env -u MAC PATH="$SCRATCH/bin:$PATH" TSKIT_MAC=builder@192.168.1.65 MAC_DIR=moonrise-release \
  bash "$C/scripts/build-tailscalekit.sh" >/dev/null 2>&1
expect_in "ssh builder@192.168.1.65 DEST=\$HOME/moonrise-release/" "$(cat "$LOG")" "build-tailscalekit.sh keeps the given Mac and folder"
expect_not_in "$ME" "$(cat "$LOG")" "build-tailscalekit.sh never mac.env's TSKIT_MAC"
rm -f "$C/scripts/mac.env"
done_testing
