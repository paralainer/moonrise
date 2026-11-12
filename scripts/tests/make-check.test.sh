#!/usr/bin/env bash
# The Makefile, scripts/check.sh, scripts/server-dev.sh, scripts/sim-create.sh and the local
# mode of scripts/common.sh (MAC=local on a Mac). Offline: docker, ssh, rsync and the scripts that
# check.sh runs are stubs.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
mkdir -p "$SCRATCH/bin"
CALLS="$SCRATCH/calls"

# --- Makefile: `make` alone is `make help`, which lists every target; each target has a line.
if command -v make >/dev/null; then
  out=$(make -s -C "$REPO" 2>&1); rc=$?
  expect_status 0 $rc "make (help by default)" "$out"
  phony=$(sed -n 's/^\.PHONY: *//p' "$REPO/Makefile")
  for t in $phony; do
    grep -qE "^$t:" "$REPO/Makefile" && ok || fail "make: .PHONY names $t but there's no rule"
    expect_in "  $t " "$out" "make help lists $t"
  done
  for t in $(sed -n 's/^\([a-z-]*\):.*/\1/p' "$REPO/Makefile"); do
    grep -qw "$t" <<<"$phony" && ok || fail "make: target $t isn't in .PHONY (nor in make help)"
  done
  out=$(make -s -C "$REPO" export 2>&1); rc=$?
  [ $rc -ne 0 ] && ok || fail "make export without DIR should fail"
  expect_in "DIR=" "$out" "make export says to set DIR"
  out=$(make -n -C "$REPO" test PARTS="server android" 2>&1)
  expect_in "scripts/test-all.sh server android" "$out" "make test PARTS=..."
  out=$(SERVER_IMAGE=moonrise-server:mine make -n -C "$REPO" server 2>&1)
  expect_in "docker build -t moonrise-server:mine server" "$out" "make server SERVER_IMAGE from the environment"
else
  echo "make-check.test.sh: no make here, Makefile checks not run"
fi

# --- check.sh, in a scratch copy: the scripts it runs are stubs that note their call.
C=$(fresh_clone)
mkdir -p "$C/server"
stub() {   # stub <path> <exit code> [output]
  printf '#!/bin/sh\necho "%s $*" >> "%s"\nprintf "%%s\\n" "%s"\nexit %s\n' "$(basename "$1")" "$CALLS" "${3:-}" "$2" > "$1"
  chmod +x "$1"
}
stub "$C/scripts/test-all.sh" 0 "==> summary (1 s)
  ok       scripts: shell tests
  SKIPPED  apple: MoonriseTests"
stub "$C/scripts/export-public.sh" 0
stub "$C/scripts/rust-dev.sh" 0
stub "$C/scripts/api-lint.sh" 0
stub "$C/scripts/build.sh" 0
stub "$SCRATCH/bin/docker" 0
printf '#!/bin/sh\necho "ssh $*" >> "%s"\nexit 255\n' "$CALLS" > "$SCRATCH/bin/ssh"; chmod +x "$SCRATCH/bin/ssh"
run() { env -u MAC -u DEV -u CHECK_IMAGE -u KEEP_IMAGE -u MAC_TEST_DIR MOONRISE_UNAME=Linux PATH="$SCRATCH/bin:$PATH" "$@" 2>&1; }

out=$(run bash "$C/scripts/check.sh" --help); rc=$?
expect_status 0 $rc "check.sh --help" "$out"
expect_in "--quick" "$out" "check.sh --help describes --quick"
out=$(run bash "$C/scripts/check.sh" --nope); rc=$?
expect_status 2 $rc "check.sh with an unknown option" "$out"

: > "$CALLS"
out=$(run bash "$C/scripts/check.sh" --quick); rc=$?
expect_status 0 $rc "check.sh --quick, all passing" "$out"
expect_in "  ok       scripts: shell tests" "$out" "test-all's summary lines are in check's"
expect_in "  SKIPPED  apple: MoonriseTests" "$out" "test-all's skips are in check's"
expect_in "SKIPPED  format: cargo fmt --check (no rustfmt.toml yet)" "$out" "no rustfmt.toml: format skipped"
expect_in "SKIPPED  image: server image (--quick)" "$out" "--quick skips the image"
expect_in "ok       export: export-public.sh" "$out" "export runs"
expect_in "ok       api: openapi.yaml valid" "$out" "--quick validates the API spec"
expect_in "api-lint.sh" "$(cat "$CALLS")" "check.sh runs api-lint.sh"
expect_not_in "docker" "$(cat "$CALLS")" "--quick builds no image"

: > "$CALLS"
touch "$C/server/rustfmt.toml"
out=$(run bash "$C/scripts/check.sh"); rc=$?
expect_status 0 $rc "check.sh, all passing, no build Mac" "$out"
expect_in "rust-dev.sh cargo fmt --check" "$(cat "$CALLS")" "with rustfmt.toml: cargo fmt --check"
expect_in "docker build -q -t moonrise-server:check" "$(cat "$CALLS")" "the image is built"
expect_in "docker image rm moonrise-server:check" "$(cat "$CALLS")" "and untagged afterwards"
expect_in "SKIPPED  apple: simulator builds (no build Mac)" "$out" "no build Mac: Apple builds skipped"
expect_not_in "build.sh" "$(cat "$CALLS")" "no build Mac: build.sh not run"

: > "$CALLS"
out=$(run env MAC=nobody@example.invalid bash "$C/scripts/check.sh"); rc=$?
expect_status 0 $rc "check.sh with an unreachable Mac" "$out"
expect_in "SKIPPED  apple: simulator builds (Mac not reachable)" "$out" "unreachable Mac: skipped"

stub "$C/scripts/test-all.sh" 1 "==> summary (1 s)
  FAILED   server: clippy (exit 1)"
out=$(run bash "$C/scripts/check.sh" --quick); rc=$?
expect_status 1 $rc "check.sh when a test fails" "$out"
expect_in "FAILED   server: clippy" "$out" "the failed part is in the summary"
stub "$C/scripts/test-all.sh" 0
stub "$C/scripts/api-lint.sh" 1
out=$(run bash "$C/scripts/check.sh" --quick); rc=$?
expect_status 1 $rc "check.sh when the API spec is invalid" "$out"
expect_in "FAILED   api: openapi.yaml valid" "$out" "an invalid spec in the summary"
stub "$C/scripts/api-lint.sh" 0
stub "$C/scripts/export-public.sh" 1
out=$(run bash "$C/scripts/check.sh" --quick); rc=$?
expect_status 1 $rc "check.sh when the export scan finds something" "$out"
expect_in "FAILED   export: export-public.sh" "$out" "export failure in the summary"

# --- server-dev.sh: arguments, and the address and setup code it prints (docker is a stub that
# gives a log with a code in it).
out=$(run bash "$REPO/scripts/server-dev.sh" --help); rc=$?
expect_status 0 $rc "server-dev.sh --help" "$out"
out=$(run bash "$REPO/scripts/server-dev.sh" nope); rc=$?
expect_status 2 $rc "server-dev.sh with an unknown command" "$out"
cat > "$SCRATCH/bin/docker" <<EOF
#!/bin/sh
echo "docker \$*" >> "$CALLS"
case "\$*" in
  *" logs "*) printf '\033[33m WARN\033[0m moonrise_server::admin::claim:   Moonrise server setup code: 123 456   (no accounts yet)\n'
              printf ' WARN moonrise_server::admin::claim:   http://172.17.0.5:8097/ (from the local network).\n' ;;
esac
EOF
chmod +x "$SCRATCH/bin/docker"
out=$(MOONRISE_DEV_HOST=192.0.2.10 MOONRISE_DEV_PORT=18123 COMPOSE_PROJECT_NAME=t run bash "$REPO/scripts/server-dev.sh" url); rc=$?
expect_status 0 $rc "server-dev.sh url" "$out"
expect_in "http://192.0.2.10:18123/" "$out" "the host's address, not the container's"
expect_in "http://localhost:18123/" "$out" "and localhost"
expect_in "setup code:          123 456" "$out" "the setup code from the log"
expect_not_in "172.17.0.5" "$out" "no container address"
expect_in "-f $REPO/server/compose.dev.yml" "$(cat "$CALLS")" "it uses server/compose.dev.yml"

# --- compose.dev.yml builds from source and doesn't pull the unpublished image.
grep -qE '^ +build: \.$' "$REPO/server/compose.dev.yml" && ok || fail "compose.dev.yml: no build: ."
expect_not_in "ghcr.io" "$(cat "$REPO/server/compose.dev.yml")" "compose.dev.yml pulls no image"

# --- sim-create.sh: arguments; without a Mac it stops before any ssh.
: > "$CALLS"
out=$(run bash "$C/scripts/sim-create.sh" --help); rc=$?
expect_status 0 $rc "sim-create.sh --help" "$out"
out=$(run bash "$C/scripts/sim-create.sh" watch); rc=$?
expect_status 2 $rc "sim-create.sh with an unknown kind" "$out"
out=$(run bash "$C/scripts/sim-create.sh" phone); rc=$?
expect_status 1 $rc "sim-create.sh without a build Mac" "$out"
expect_in "no build Mac" "$out" "sim-create.sh says why"
expect_not_in "ssh" "$(cat "$CALLS")" "no ssh without a Mac"

# --- Local mode: on macOS with no MAC, MAC=local; ssh runs here, rsync and scp copy here.
out=$(env -u MAC MOONRISE_UNAME=Darwin bash -c '. "$1/scripts/common.sh"; echo "mac=[$MAC]"' _ "$C" 2>&1)
expect_in "mac=[local]" "$out" "macOS without MAC: local mode"
out=$(env -u MAC MOONRISE_UNAME=Linux bash -c '. "$1/scripts/common.sh"; echo "mac=[$MAC]"' _ "$C" 2>&1)
expect_in "mac=[]" "$out" "Linux without MAC: no Mac"
out=$(env -u MAC MOONRISE_UNAME=Darwin MAC=me@mac.example bash -c '. "$1/scripts/common.sh"; type -t ssh' _ "$C" 2>&1)
expect_in "file" "$out" "an explicit MAC keeps the real ssh"
H="$SCRATCH/home"; mkdir -p "$H/build-here"
out=$(HOME="$H" MAC=local PATH="$SCRATCH/bin:$PATH" bash -c '. "$1/scripts/common.sh"
  ssh -o ConnectTimeout=5 -o BatchMode=yes "$MAC" true && echo reachable
  echo "x y" | ssh "$MAC" "cd ~/build-here && read -r a b && echo \"in \$(basename \$PWD): \$b\""
  ssh -q "$MAC" "echo \$((1 + 2))"' _ "$C" 2>&1)
expect_in "reachable" "$out" "local ssh: need_mac_reachable's check passes"
expect_in "in build-here: y" "$out" "local ssh: runs in a shell from home, stdin passed on"
expect_in "3" "$out" "local ssh: options skipped"
expect_not_in "ssh " "$(cat "$CALLS")" "local mode never calls the real ssh"
mkdir -p "$SCRATCH/src/sub"; echo hi > "$SCRATCH/src/sub/f"
out=$(HOME="$H" MAC=local bash -c '. "$1/scripts/common.sh"
  rsync -a --delete --exclude "build-*" "$2/" "$MAC:~/copy/" && cat ~/copy/sub/f
  echo shot > "$3/shot.png"; scp -q "$MAC:$3/shot.png" "$3/got.png" && cat "$3/got.png"' _ "$C" "$SCRATCH/src" "$H" 2>&1)
expect_in "hi" "$out" "local rsync to local:~/copy"
expect_in "shot" "$out" "local scp from local:<path>"
# The checkout itself as the destination: nothing is copied or deleted.
mkdir -p "$H/clone"; cp -R "$C/." "$H/clone/"; touch "$H/clone/keep-me"
out=$(HOME="$H" MAC=local MAC_DIR=clone bash -c '. "$1/scripts/common.sh"
  rsync -a --delete "$2/" "$MAC:~/$MAC_DIR/" && echo done' _ "$H/clone" "$SCRATCH/src" 2>&1)
expect_in "done" "$out" "rsync onto the checkout: skipped"
[ -e "$H/clone/keep-me" ] && [ ! -e "$H/clone/sub" ] && ok || fail "rsync onto the checkout changed it"
# With no MAC_DIR, local mode builds in the checkout (under the home folder), or in
# ~/moonrise-build for a checkout elsewhere; never in some other ~/video-player.
out=$(env -u MAC_DIR HOME="$H" MAC=local bash -c '. "$1/scripts/common.sh"; echo "dir=[$MAC_DIR]"' _ "$H/clone" 2>&1)
expect_in "dir=[clone]" "$out" "local mode: the checkout itself"
out=$(env -u MAC_DIR HOME="$H" MAC=local bash -c '. "$1/scripts/common.sh"; echo "dir=[$MAC_DIR]"' _ "$C" 2>&1)
expect_in "dir=[moonrise-build]" "$out" "local mode, checkout outside home: ~/moonrise-build"
out=$(env -u MAC_DIR HOME="$H" MAC=me@mac.example bash -c '. "$1/scripts/common.sh"; echo "dir=[$MAC_DIR]"' _ "$C" 2>&1)
expect_in "dir=[video-player]" "$out" "a remote Mac: ~/video-player as before"
# A copy never goes into a folder that exists and isn't an earlier build copy.
mkdir -p "$H/video-player"; echo mine > "$H/video-player/notes.txt"
out=$(HOME="$H" MAC=local MAC_DIR=video-player bash -c '. "$1/scripts/common.sh"
  rsync -a --delete "$2/" "$MAC:~/$MAC_DIR/" && echo copied' _ "$C" "$SCRATCH/src" 2>&1); rc=$?
expect_in "isn't a build copy" "$out" "an unrelated folder: refused"
expect_not_in "copied" "$out" "an unrelated folder: nothing copied"
[ -e "$H/video-player/notes.txt" ] && [ ! -e "$H/video-player/sub" ] && ok || fail "the unrelated folder was changed"
mkdir -p "$H/other/.git" "$H/other/scripts"; touch "$H/other/scripts/common.sh" "$H/other/mine"
out=$(HOME="$H" MAC=local MAC_DIR=other bash -c '. "$1/scripts/common.sh"
  rsync -a --delete "$2/" "$MAC:~/$MAC_DIR/" && echo copied' _ "$C" "$SCRATCH/src" 2>&1)
expect_in "isn't a build copy" "$out" "another checkout: refused"
[ -e "$H/other/mine" ] && ok || fail "the other checkout was changed"
mkdir -p "$H/old-copy/scripts"; touch "$H/old-copy/scripts/common.sh" "$H/old-copy/stale"
out=$(HOME="$H" MAC=local MAC_DIR=old-copy bash -c '. "$1/scripts/common.sh"
  rsync -a --delete "$2/" "$MAC:~/$MAC_DIR/" && echo copied' _ "$C" "$SCRATCH/src" 2>&1)
expect_in "copied" "$out" "an earlier build copy: updated"
[ -e "$H/old-copy/sub/f" ] && [ ! -e "$H/old-copy/stale" ] && ok || fail "the earlier build copy wasn't updated"
done_testing
