#!/usr/bin/env bash
# Every automatic test, in one command: run it before opening a PR.
#
#   scripts/test-all.sh                 # scripts, server, Android, Windows core, Apple
#   scripts/test-all.sh server android  # only some of them
#   scripts/test-all.sh smb             # the Samba integration test (not run by default)
#
#   scripts  scripts/tests/run.sh: the shell scripts' own tests (fresh clone, deploy installs,
#            the Android TV deploy, DEV=1, export-public), in scratch folders with ssh stubbed
#   server   cargo test --locked and cargo clippy --all-targets -D warnings, in Docker
#            (scripts/rust-dev.sh: the toolchain pinned in server/rust-toolchain.toml, image
#            moonrise-rust-dev:<version>, built on first use)
#   android  clients/android/build.sh test (the JVM unit tests), in Docker
#   windows  scripts/test-windows-core.sh: the Windows app's shared rules (Moonrise.Core), in Docker
#   apple    scripts/test-apple.sh: MoonriseTests on a throwaway simulator on the build Mac, and a
#            compile of the macOS app; skipped (with a note) when no build Mac is set or it can't
#            be reached
#   smb      server/tools/smb-test.sh: the server against a Samba server in Docker (the #[ignore]d
#            smb_* tests). Only when asked for: it needs the moonrise-server image (ffmpeg,
#            for the fixture) and the Samba image locally, and is skipped (with a note) without them.
#            SMB_NAME and SMB_PORT set its container names and port (moonrise-smbtest, 1445)
#
# All of it is offline: no production server, no devices, no network. Exits non-zero when anything
# fails; a skipped part doesn't fail the run. Regression tests per past issue: the regression map
# in docs/dev/testing.md.
#
# CARGO_TARGET_VOLUME names the Docker volume the server builds into (default moonrise-test-target),
# so a test run doesn't share a target folder with a build running elsewhere. CARGO_VOLUME (cargo's
# registry cache, default moonrise-cargo) and RUST_DEV_IMAGE are passed on to scripts/rust-dev.sh.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
case "${1:-}" in
  -h|--help) awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }' "$0"; exit 0 ;;
esac
PARTS=("$@")
[ ${#PARTS[@]} -eq 0 ] && PARTS=(scripts server android windows apple)
CARGO_TARGET_VOLUME=${CARGO_TARGET_VOLUME:-moonrise-test-target}
export CARGO_TARGET_VOLUME

declare -a SUMMARY=()
failed=0
started=$(date +%s)

# record <part> <exit code> [skip code]: adds a line to the summary.
record() {
  local part=$1 rc=$2 skip=${3:-}
  if [ "$rc" -eq 0 ]; then SUMMARY+=("  ok       $part")
  elif [ -n "$skip" ] && [ "$rc" -eq "$skip" ]; then SUMMARY+=("  SKIPPED  $part")
  else SUMMARY+=("  FAILED   $part (exit $rc)"); failed=1
  fi
}

scripts() {
  echo "==> scripts: scripts/tests/run.sh"
  "$ROOT/scripts/tests/run.sh"
  record "scripts: shell tests" $?
}

server() {
  echo "==> server: cargo test --locked"
  "$ROOT/scripts/rust-dev.sh" cargo test --locked 2>&1 \
    | grep -E '^test result|FAILED|panicked|^error|^failures:|^    [a-z_:]+$|^==> building|^ERROR'
  record "server: cargo test" "${PIPESTATUS[0]}"
  echo "==> server: cargo clippy --all-targets -D warnings"
  "$ROOT/scripts/rust-dev.sh" cargo clippy --locked --all-targets -- -D warnings 2>&1 \
    | grep -vE '^\s+(Compiling|Checking) ' | tail -30
  record "server: clippy" "${PIPESTATUS[0]}"
}

windows() {
  echo "==> windows: scripts/test-windows-core.sh"
  "$ROOT/scripts/test-windows-core.sh" 2>&1 | tail -12
  record "windows: core tests" "${PIPESTATUS[0]}"
}

android() {
  echo "==> android: build.sh test"
  "$ROOT/clients/android/build.sh" test 2>&1 | grep -vE '^> Task .*(UP-TO-DATE|NO-SOURCE|SKIPPED)$' | tail -30
  local rc=${PIPESTATUS[0]}
  local results="$ROOT/clients/android/core/build/test-results/test"
  if [ -d "$results" ]; then
    echo "    $(cat "$results"/*.xml | grep -o '<testsuite [^>]*' | sed -E 's/.*tests="([0-9]+)".*failures="([0-9]+)".*errors="([0-9]+)".*/\1 \2 \3/' \
      | awk '{t+=$1; f+=$2+$3} END {print t " tests, " f " failed"}')"
  fi
  record "android: unit tests" "$rc"
}

apple() {
  "$ROOT/scripts/test-apple.sh"
  record "apple: MoonriseTests" $? 3
}

smb() {
  echo "==> smb: server/tools/smb-test.sh"
  local img
  for img in moonrise-server:latest ghcr.io/servercontainers/samba:latest alpine:3; do
    if ! docker image inspect "$img" >/dev/null 2>&1; then
      echo "    no $img image here: Samba test not run"
      record "smb: Samba integration" 3 3
      return
    fi
  done
  TARGET_VOLUME="$CARGO_TARGET_VOLUME" \
    PORT="${SMB_PORT:-1445}" "$ROOT/server/tools/smb-test.sh" 2>&1 | grep -vE '^\s+(Compiling|Downloaded|Downloading) '
  record "smb: Samba integration" "${PIPESTATUS[0]}"
}

for part in "${PARTS[@]}"; do
  case "$part" in
    scripts|server|android|windows|apple|smb) "$part" ;;
    *) echo "unknown part: $part (scripts, server, android, apple, smb)" >&2; exit 2 ;;
  esac
done

echo
echo "==> summary ($(( $(date +%s) - started )) s)"
printf '%s\n' "${SUMMARY[@]}"
exit $failed
