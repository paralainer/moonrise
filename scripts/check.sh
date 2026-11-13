#!/usr/bin/env bash
# The local check, in place of CI: everything a pull request must pass (`make check`).
#
#   scripts/check.sh            # all of it
#   scripts/check.sh --quick    # without the server image and the Apple app builds
#
#   tests    scripts/test-all.sh: the scripts' tests, the server's cargo test and clippy, the
#            Android unit tests, the Apple unit tests and macOS compile (on the build Mac; skipped
#            with a note when there is none)
#   format   cargo fmt --check in the dev container, once the server has a rustfmt.toml (until
#            then: skipped with a note)
#   api      scripts/api-lint.sh: api/openapi.yaml and its examples are valid (Redocly CLI, pinned,
#            in Docker); the server's tests check that it names every route
#   image    docker build of the server image (server/Dockerfile), tagged CHECK_IMAGE (default
#            moonrise-server:check) and untagged again afterwards (KEEP_IMAGE=1 keeps it)
#   apple    scripts/build.sh sim and tvos: the iPhone and Apple TV simulator builds, on the build
#            Mac in ~/$MAC_TEST_DIR (default moonrise-tests, as scripts/test-apple.sh), so they don't
#            touch a folder you build in. Skipped (with a note) when no build Mac is set, it can't
#            be reached or it is listed in PERSONAL_MACS; on a Mac itself they run locally (MAC=local)
#   export   scripts/export-public.sh into a scratch folder (removed afterwards): the committed
#            tree (HEAD) has no secrets or private details
#
# Prints a summary and exits non-zero when anything failed; a skipped part doesn't fail the run.
# The settings test-all.sh takes (CARGO_TARGET_VOLUME, CARGO_VOLUME, RUST_DEV_IMAGE, MAC,
# MAC_TEST_DIR) apply here too.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
QUICK=
case "${1:-}" in
  -h|--help) awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }' "$0"; exit 0 ;;
  --quick) QUICK=1 ;;
  "") ;;
  *) echo "usage: $0 [--quick]" >&2; exit 2 ;;
esac
MAC_TEST_DIR=${MAC_TEST_DIR:-moonrise-tests}
export MAC_TEST_DIR
CHECK_IMAGE=${CHECK_IMAGE:-moonrise-server:check}

declare -a SUMMARY=()
failed=0
started=$(date +%s)
LOG=$(mktemp "${TMPDIR:-/tmp}/moonrise-check.XXXXXX")
trap 'rm -f "$LOG"' EXIT

record() {   # record <part> <exit code>: 0 ok, 3 skipped, else failed
  if [ "$2" -eq 0 ]; then SUMMARY+=("  ok       $1")
  elif [ "$2" -eq 3 ]; then SUMMARY+=("  SKIPPED  $1")
  else SUMMARY+=("  FAILED   $1 (exit $2)"); failed=1
  fi
}

# tests: test-all.sh, its own summary lines folded into this one.
echo "==> tests: scripts/test-all.sh"
"$ROOT/scripts/test-all.sh" 2>&1 | tee "$LOG"
rc=${PIPESTATUS[0]}
parts=$(sed -n '/^==> summary/,$p' "$LOG" | grep -E '^  (ok|SKIPPED|FAILED) ')
if [ -n "$parts" ]; then
  while IFS= read -r line; do SUMMARY+=("$line"); done <<<"$parts"
  [ "$rc" -eq 0 ] || failed=1
else
  record "tests: scripts/test-all.sh" "$rc"
fi

echo "==> format: cargo fmt --check"
if [ -f "$ROOT/server/rustfmt.toml" ] || [ -f "$ROOT/server/.rustfmt.toml" ]; then
  "$ROOT/scripts/rust-dev.sh" cargo fmt --check
  record "format: cargo fmt --check" $?
else
  echo "    the server has no rustfmt.toml yet: formatting not checked"
  record "format: cargo fmt --check (no rustfmt.toml yet)" 3
fi

echo "==> api: scripts/api-lint.sh (api/openapi.yaml)"
"$ROOT/scripts/api-lint.sh"
record "api: openapi.yaml valid (api-lint.sh)" $?

if [ -n "$QUICK" ]; then
  record "image: server image (--quick)" 3
  record "apple: simulator builds (--quick)" 3
else
  echo "==> image: docker build server/ ($CHECK_IMAGE)"
  docker build -q -t "$CHECK_IMAGE" "$ROOT/server"
  rc=$?
  record "image: server image builds" $rc
  if [ $rc -eq 0 ] && [ "${KEEP_IMAGE:-}" != 1 ]; then docker image rm "$CHECK_IMAGE" >/dev/null; fi

  echo "==> apple: simulator builds (scripts/build.sh sim, tvos) in ~/$MAC_TEST_DIR"
  mac=$(bash -c '. "$1/scripts/common.sh" 2>/dev/null; printf "%s" "$MAC"' _ "$ROOT")
  if [ -z "$mac" ]; then
    echo "    no build Mac (MAC= or scripts/mac.env): Apple builds not run"
    record "apple: simulator builds (no build Mac)" 3
  elif ! bash -c '. "$1/scripts/common.sh" 2>/dev/null; test_mac_ok' _ "$ROOT"; then
    echo "    $mac is a personal Mac (PERSONAL_MACS): Apple builds not run"
    record "apple: simulator builds (personal Mac)" 3
  elif ! bash -c '. "$1/scripts/common.sh" 2>/dev/null; mac_reachable' _ "$ROOT"; then
    echo "    can't reach the build Mac $mac: Apple builds not run"
    record "apple: simulator builds (Mac not reachable)" 3
  else
    for p in sim tvos; do
      MAC_DIR=$MAC_TEST_DIR "$ROOT/scripts/build.sh" "$p"
      record "apple: build.sh $p" $?
    done
  fi
fi

echo "==> export: scripts/export-public.sh (the committed tree)"
EXPORT_DIR=$(mktemp -d "${TMPDIR:-/tmp}/moonrise-check-export.XXXXXX")
"$ROOT/scripts/export-public.sh" "$EXPORT_DIR" 2>&1 | tail -15
record "export: export-public.sh" "${PIPESTATUS[0]}"
rm -rf "$EXPORT_DIR"

echo
echo "==> check summary ($(( $(date +%s) - started )) s)"
printf '%s\n' "${SUMMARY[@]}"
exit $failed
