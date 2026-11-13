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
# The build Mac is a stub `ssh` that answers as fastlane would.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
C=$(fresh_clone)
mkdir -p "$C/clients/apple/fastlane" "$SCRATCH/bin"
cp -r "$REPO/clients/apple/fastlane/metadata" "$C/clients/apple/fastlane/"
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
run() { rm -f "$LOG"; touch "$LOG"; PATH="$SCRATCH/bin:$PATH" MAC=nobody@example.invalid ASC_ENV='~/asc.env' bash "$C/scripts/release-apple.sh" "$@" 2>&1; }

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

rm "$M/review_information/notes.txt"
out=$(run check-metadata); rc=$?
expect_status 1 $rc "no review notes" "$out"
restore

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
done_testing
