#!/usr/bin/env bash
# The deploy scripts said a device install worked when it hadn't. install_on_device
# (scripts/common.sh) must exit 1 on a failed devicectl install, on one that gives no install
# record, and on one that installed another bundle id; it succeeds only on a confirmed install.
# The build Mac is a stub `ssh` that answers as devicectl would (STUB=<case>). Also build_stamp:
# the commit, with -dirty for uncommitted changes to tracked files.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
C=$(fresh_clone)
mkdir -p "$SCRATCH/bin"
cat > "$SCRATCH/bin/ssh" <<'STUB'
#!/bin/bash
# ssh <mac> <command>: canned answers by what the command asks for.
cmd=$*
case $cmd in
  *PlistBuddy*) printf '20261108\nabc1234\n' ;;
  *"device install app"*)
    case $STUB in
      ok) printf 'App installed:\n• bundleID: org.example.moonrise\n• databaseSequenceNumber: 42\n' ;;
      error) echo 'ERROR: The device rejected the install (connection reset)'; exit 1 ;;
      norecord) echo 'Install started' ;;
      other) printf '• bundleID: com.example.other\n• databaseSequenceNumber: 7\n' ;;
      oldbuild) printf '• bundleID: org.example.moonrise\n• databaseSequenceNumber: 9\n' ;;
    esac ;;
  *"device info apps"*) [ "$STUB" = oldbuild ] && echo 20250101 || echo 20261108 ;;
  *) exit 0 ;;
esac
STUB
chmod +x "$SCRATCH/bin/ssh"
install() {
  STUB=$1 PATH="$SCRATCH/bin:$PATH" bash -c 'set -e; MAC=nobody@example.invalid; . "$1/scripts/common.sh"; sleep() { :; }; install_on_device DEVICE "the test phone" build/Moonrise.app' _ "$C" 2>&1
}

out=$(install ok); rc=$?
expect_status 0 $rc "a confirmed install" "$out"
expect_in "now has Moonrise abc1234 (build 20261108" "$out" "the build stamp and number are printed"
out=$(install error); rc=$?
expect_status 1 $rc "devicectl failed" "$out"
expect_in "install on the test phone failed" "$out" "says it failed"
out=$(install norecord); rc=$?
expect_status 1 $rc "no install record" "$out"
out=$(install other); rc=$?
expect_status 1 $rc "another bundle id installed" "$out"
expect_in "not org.example.moonrise" "$out" "names the bundle id"
out=$(install oldbuild); rc=$?
expect_status 1 $rc "the device still lists the old build" "$out"

# build_stamp: the commit, -dirty with uncommitted changes to tracked files (untracked don't count).
G="$SCRATCH/stamp"; mkdir -p "$G/scripts"; cp "$C/scripts/common.sh" "$G/scripts/"; mkdir -p "$G/clients/apple/Config"; cp "$C/clients/apple/Config/Signing.xcconfig" "$G/clients/apple/Config/"
git -C "$G" init -q && git -C "$G" add -A && git -C "$G" -c user.name=t -c user.email=t@example.com -c commit.gpgsign=false commit -qm t
sha=$(git -C "$G" rev-parse --short HEAD)
stamp() { bash -c '. "$1/scripts/common.sh" 2>/dev/null; build_stamp' _ "$G"; }
[ "$(stamp)" = "$sha" ] && ok || fail "build_stamp on a clean tree: $(stamp), wanted $sha"
touch "$G/untracked"; [ "$(stamp)" = "$sha" ] && ok || fail "an untracked file made the stamp dirty"
echo "# x" >> "$G/clients/apple/Config/Signing.xcconfig"; [ "$(stamp)" = "$sha-dirty" ] && ok || fail "build_stamp with a change: $(stamp), wanted $sha-dirty"
done_testing
