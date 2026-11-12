# Sourced by the script tests (scripts/tests/*.test.sh): a scratch folder, assertions, a summary.
# Each test runs the real script in a scratch copy, with stubs for anything outside this machine
# (ssh, the build Mac, devices), so nothing leaves the machine.
set -uo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRATCH=$(mktemp -d "${TMPDIR:-/tmp}/moonrise-script-test.XXXXXX")
trap 'rm -rf "$SCRATCH"' EXIT
FAILED=0
PASSED=0

# ok <description>: the last check passed. fail <description> [detail]: it didn't.
ok() { PASSED=$((PASSED + 1)); }
fail() { FAILED=$((FAILED + 1)); echo "FAIL $(basename "$0"): $1"; [ -z "${2:-}" ] || printf '%s\n' "$2" | sed 's/^/     | /' | head -20; }

# expect_status <want> <got> <description> [output]
expect_status() { if [ "$1" = "$2" ]; then ok; else fail "$3: exit $2, wanted $1" "${4:-}"; fi; }
# expect_in <needle> <haystack> <description>; expect_not_in likewise.
expect_in() { if grep -qF -- "$1" <<<"$2"; then ok; else fail "$3: no \"$1\" in the output" "$2"; fi; }
expect_not_in() { if grep -qF -- "$1" <<<"$2"; then fail "$3: \"$1\" is in the output" "$2"; else ok; fi; }

# A scratch copy of scripts/ (and a public clients/apple/Config/Signing.xcconfig), as a fresh clone
# has them: no scripts/mac.env, no scripts/local, no clients/apple/Config/Signing.local.xcconfig.
# Prints its root.
fresh_clone() {
  local d="$SCRATCH/clone-$RANDOM"
  mkdir -p "$d/scripts" "$d/clients/apple/Config"
  cp "$REPO"/scripts/*.sh "$REPO/scripts/mac.env.example" "$d/scripts/"
  printf 'MOONRISE_BUNDLE_ID = org.example.moonrise\nMOONRISE_DEVELOPMENT_TEAM =\n' > "$d/clients/apple/Config/Signing.xcconfig"
  cp "$REPO/clients/apple/Config/toolchain.env" "$d/clients/apple/Config/"
  printf '%s\n' "$d"
}

done_testing() {
  echo "$(basename "$0"): $PASSED passed, $FAILED failed"
  [ "$FAILED" = 0 ]
}
