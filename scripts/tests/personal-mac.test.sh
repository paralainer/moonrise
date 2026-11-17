#!/usr/bin/env bash
# A Mac listed in PERSONAL_MACS (someone's own Mac) is refused by every script that runs anything
# on MAC or TSKIT_MAC, before any ssh or rsync, with a message that says why: the builds
# (build.sh), simulators (sim.sh, tvsim.sh, sim-create.sh), screenshots and input (shot.sh,
# tvshot.sh, tap.sh, tvkey.sh), Dev app installs (DEV=1 deploy-mac.sh, deploy-phone.sh,
# deploy-tv.sh), TailscaleKit (build-tailscalekit.sh) and the tests (test-apple.sh).
# The one way onto a personal Mac is an owner install: deploy-mac.sh (and deploy-phone.sh,
# deploy-tv.sh) without DEV=1, installing the app itself; the build.sh it runs goes through too.
# A build Mac not in the list works as before.
# The Macs are a stub `ssh`/`rsync`/`scp` that log each call.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
C=$(fresh_clone)
mkdir -p "$SCRATCH/bin"
printf 'MOONRISE_BUNDLE_ID = org.example.mine\nMOONRISE_DEVELOPMENT_TEAM = ABCDE12345\n' > "$C/clients/apple/Config/Signing.local.xcconfig"
LOG="$SCRATCH/calls.log"; export LOG
printf '#!/bin/bash\necho "rsync $*" >> "$LOG"\n' > "$SCRATCH/bin/rsync"
printf '#!/bin/bash\necho "scp $*" >> "$LOG"\n' > "$SCRATCH/bin/scp"
# ssh: the built app's version and stamp when asked (deploy-mac.sh checks them), else nothing.
cat > "$SCRATCH/bin/ssh" <<'STUB'
#!/bin/bash
echo "ssh $*" >> "$LOG"
case $* in
  *MoonriseBuildStamp*) printf '20261108\nabc1234\n' ;;
  *PlistBuddy*) echo 20261108 ;;
esac
exit 0
STUB
chmod +x "$SCRATCH/bin/"*

ME=owner@personal-mac.example
BUILDER=builder@build-mac.example
# mac.env as a personal one may be set up: MAC at someone's own Mac, which is listed.
printf 'MAC=${MAC:-%s}\nTSKIT_MAC=${TSKIT_MAC:-$MAC}\nPERSONAL_MACS="personal-mac.example me@studio.example"\nSIM_PHONE=PHONE-SIM\nSIM_TV=TV-SIM\nPHONE_DEVICE=PHONE\nTV_DEVICE=TV\n' "$ME" > "$C/scripts/mac.env"
# run [NAME=value...] <script> [args]: the script with the stubs; mac.env's settings, except the
# ones given here.
run() {
  rm -f "$LOG"; touch "$LOG"
  local vars=()
  while [[ $1 == *=* ]]; do vars+=("$1"); shift; done
  local s=$1; shift
  env -u MAC -u TSKIT_MAC -u DEV -u PERSONAL_MACS -u MAC_DIR -u MOONRISE_RELEASE_DIR -u MOONRISE_OWNER_INSTALL \
    -u MOONRISE_STORE_BUILD PATH="$SCRATCH/bin:$PATH" "${vars[@]}" bash "$C/scripts/$s" "$@" 2>&1
}
reached() { [ -s "$LOG" ]; }
refused() {  # refused <description> <output> <exit status>
  [ "$3" != 0 ] && ok || fail "$1: exit 0 on a personal Mac" "$2"
  expect_in "is listed in PERSONAL_MACS" "$2" "$1 says why"
  reached && fail "$1 reached a Mac" "$(cat "$LOG")" || ok
}

# Every script that runs on MAC, with mac.env's MAC (personal) and with one from the environment.
for args in "build.sh sim" "build.sh tvos" "build.sh mac" "build.sh ios" "sim.sh" "tvsim.sh" \
    "sim-create.sh" "sim-create.sh tv" "shot.sh" "tvshot.sh" "tap.sh 10 10" "tvkey.sh up" "test-apple.sh"; do
  out=$(run $args); rc=$?
  refused "$args (mac.env's MAC)" "$out" $rc
  out=$(run MAC=someone@STUDIO.example $args); rc=$?
  refused "$args (MAC from the environment)" "$out" $rc
done
# Dev app installs are testing, so a personal Mac is refused for them.
for s in deploy-mac.sh deploy-phone.sh deploy-tv.sh; do
  out=$(run DEV=1 $s); rc=$?
  refused "DEV=1 $s" "$out" $rc
done
# An owner install from outside a deploy script doesn't make a personal Mac a build Mac for DEV=1.
out=$(run DEV=1 MOONRISE_OWNER_INSTALL=1 build.sh sim); rc=$?
refused "DEV=1 build.sh with MOONRISE_OWNER_INSTALL=1" "$out" $rc
# TailscaleKit: TSKIT_MAC is checked too, whatever MAC is.
out=$(run MAC=$BUILDER TSKIT_MAC=$ME build-tailscalekit.sh); rc=$?
refused "build-tailscalekit.sh with a personal TSKIT_MAC" "$out" $rc

# The build Mac: builds and the rest run there as before.
for args in "build.sh sim" "build.sh tvos" "sim-create.sh" "build-tailscalekit.sh"; do
  out=$(run MAC=$BUILDER TSKIT_MAC=$BUILDER $args); rc=$?
  expect_not_in "is listed in PERSONAL_MACS" "$out" "$args on the build Mac isn't refused"
  grep -q "$BUILDER" "$LOG" && ok || fail "$args didn't reach the build Mac" "$out"
  grep -q "$ME" "$LOG" && fail "$args reached the personal Mac" "$(cat "$LOG")" || ok
done
out=$(run MAC=$BUILDER build.sh sim); rc=$?
expect_status 0 $rc "build.sh sim on the build Mac" "$out"
out=$(run DEV=1 MAC=$BUILDER deploy-mac.sh); rc=$?
expect_status 0 $rc "DEV=1 deploy-mac.sh on the build Mac" "$out"

# The owner install: the app itself (no DEV=1) onto the personal Mac, with its build.
out=$(run deploy-mac.sh); rc=$?
expect_status 0 $rc "deploy-mac.sh (the app itself) on a personal Mac" "$out"
expect_in "an owner install" "$out" "says it is an owner install"
expect_in "rsync " "$(cat "$LOG")" "the build copied the tree"
expect_in "ssh $ME open '/Applications/Moonrise.app'" "$(cat "$LOG")" "installed and opened the app on the personal Mac"
rm -f "$C/scripts/mac.env"
done_testing
