#!/usr/bin/env bash
# On a fresh clone (no clients/apple/Config/Signing.local.xcconfig, no scripts/mac.env) sourcing
# scripts/common.sh must not exit; scripts give the public defaults and only the steps that
# need a build Mac or signing stop, each with a message saying what to set.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
C=$(fresh_clone)
# Nothing may reach for the network: ssh and rsync are stubs that note the call and fail.
mkdir -p "$SCRATCH/bin"
for t in ssh rsync; do printf '#!/bin/sh\necho "%s $*" >> "%s/calls"\nexit 255\n' "$t" "$SCRATCH" > "$SCRATCH/bin/$t"; chmod +x "$SCRATCH/bin/$t"; done
run() { env -u MAC -u DEV PATH="$SCRATCH/bin:$PATH" "$@" 2>&1; }

out=$(run bash -c '. "$1/scripts/common.sh"; echo "id=$APP_ID team=[$APP_TEAM] mac=[$MAC]"' _ "$C"); rc=$?
expect_status 0 $rc "sourcing common.sh on a fresh clone" "$out"
expect_in "id=org.example.moonrise team=[] mac=[]" "$out" "public defaults"
expect_in "no clients/apple/Config/Signing.local.xcconfig" "$out" "a note on what's missing"

out=$(run bash -c 'DEV=1; . "$1/scripts/common.sh"; echo "id=$APP_ID name=$APP_NAME"' _ "$C")
expect_in "id=org.example.moonrise.dev name=Moonrise Dev" "$out" "DEV=1 on a fresh clone"

# --help works for every script that reads common.sh (the header comment, exit 0).
for s in "$C"/scripts/*.sh; do
  grep -q 'common.sh"' "$s" || continue
  case $(basename "$s") in common.sh|build-tailscalekit.sh) continue ;; esac
  out=$(run bash "$s" --help); rc=$?
  expect_status 0 $rc "$(basename "$s") --help" "$out"
  [ -n "$out" ] && ok || fail "$(basename "$s") --help printed nothing"
done

# Steps that need the Mac stop with a message before any ssh; signing steps say what to set.
out=$(run bash "$C/scripts/build.sh" sim); rc=$?
expect_status 1 $rc "build.sh sim without a Mac" "$out"
expect_in "no build Mac" "$out" "build.sh sim says why"
out=$(run bash -c 'MAC=nobody@example.invalid; . "$1/scripts/common.sh"; need_signing "a device install"' _ "$C"); rc=$?
expect_status 1 $rc "need_signing on a fresh clone" "$out"
expect_in "clients/apple/Config/Signing.local.xcconfig" "$out" "need_signing says what to set"
out=$(run bash -c '. "$1/scripts/common.sh"; MAC=; need_mac' _ "$C"); rc=$?
expect_status 1 $rc "need_mac without a Mac" "$out"
[ ! -e "$SCRATCH/calls" ] && ok || fail "something called ssh or rsync" "$(cat "$SCRATCH/calls")"

# A git-ignored Signing.local.xcconfig left in the old place (Config/ at the root, before the
# Apple project moved to clients/apple/) stops every script with the command that moves it, rather
# than build under the public id; --help still works. Once moved, its identity is used.
mkdir -p "$C/Config"; printf 'MOONRISE_BUNDLE_ID = org.example.mine\nMOONRISE_DEVELOPMENT_TEAM = TEAM1\n' > "$C/Config/Signing.local.xcconfig"
out=$(run bash -c '. "$1/scripts/common.sh"; echo "id=$APP_ID"' _ "$C"); rc=$?
expect_status 1 $rc "Signing.local.xcconfig in the old place" "$out"
expect_in "mv '$C/Config/Signing.local.xcconfig' '$C/clients/apple/Config/'" "$out" "says how to move it"
expect_not_in "id=" "$out" "stops before anything runs"
out=$(run bash "$C/scripts/build.sh" --help); rc=$?
expect_status 0 $rc "--help with the old Signing.local.xcconfig" "$out"
mv "$C/Config/Signing.local.xcconfig" "$C/clients/apple/Config/"
out=$(run bash -c '. "$1/scripts/common.sh"; echo "id=$APP_ID team=$APP_TEAM"' _ "$C"); rc=$?
expect_status 0 $rc "Signing.local.xcconfig moved" "$out"
expect_in "id=org.example.mine team=TEAM1" "$out" "the moved identity is used"
rm "$C/clients/apple/Config/Signing.local.xcconfig"
done_testing
