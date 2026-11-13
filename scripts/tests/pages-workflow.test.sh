#!/usr/bin/env bash
# .github/workflows/pages.yml publishes site/ and nothing else: it starts only on a push to main
# that touches site/** or by hand, uploads the site folder, and runs no commands of its own. Every
# other workflow stays manual only (CONTRIBUTING.md). Also checks, on scratch copies, that each
# problem is reported.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

# The top-level block <key>: of a workflow file, comments and blank lines dropped.
block() { awk -v k="$1:" '$0 == k { on = 1; next } on && /^[^ #]/ { exit } on' "$2" | sed -e 's/[[:space:]]*#.*//' -e '/^[[:space:]]*$/d'; }

# check_workflows <dir>: prints each problem, exits 1 if any.
check_workflows() {
  local dir=$1 bad=0 f want got
  f="$dir/pages.yml"
  [ -f "$f" ] || { echo "no pages.yml"; return 1; }
  want=$(printf '  push:\n    branches: [main]\n    paths:\n      - "site/**"\n  workflow_dispatch:')
  got=$(block on "$f")
  [ "$got" = "$want" ] || { echo "pages.yml starts on something other than a push to main touching site/** or by hand:"; echo "$got"; bad=1; }
  grep -qE '^[[:space:]]*(- )?run:' "$f" && { echo "pages.yml runs commands of its own"; bad=1; }
  got=$(grep -oE 'uses: *[^ ]+' "$f" | sed -E 's/uses: *//; s/@.*//' | sort | tr '\n' ' ')
  [ "$got" = "actions/checkout actions/configure-pages actions/deploy-pages actions/upload-pages-artifact " ] ||
    { echo "pages.yml uses other actions: $got"; bad=1; }
  awk '/uses: *actions\/upload-pages-artifact@/ { u = 1; next } u && /^[[:space:]]*- / { exit } u && /^[[:space:]]+path: *site[[:space:]]*$/ { found = 1 } END { exit !found }' "$f" ||
    { echo "pages.yml doesn't upload path: site"; bad=1; }
  for f in "$dir"/*.yml; do
    [ "$(basename "$f")" = pages.yml ] && continue
    got=$(block on "$f")
    [ "$got" = "  workflow_dispatch:" ] || { echo "$(basename "$f") isn't manual only"; bad=1; }
  done
  return $bad
}

WF="$REPO/.github/workflows"
out=$(check_workflows "$WF"); rc=$?
expect_status 0 "$rc" "the Pages workflow publishes site/ only, the others are manual" "$out"

# Scratch copies with each problem in turn.
fresh() { local d="$SCRATCH/wf-$RANDOM"; mkdir -p "$d"; cp "$WF"/*.yml "$d/"; printf '%s\n' "$d"; }

d=$(fresh); sed -i 's|"site/\*\*"|"**"|' "$d/pages.yml"
out=$(check_workflows "$d"); rc=$?
expect_status 1 "$rc" "a push touching anything is reported" "$out"
expect_in "starts on something other" "$out" "trigger message"

d=$(fresh); sed -i 's|^  workflow_dispatch:|  pull_request:\n  workflow_dispatch:|' "$d/pages.yml"
out=$(check_workflows "$d"); rc=$?
expect_status 1 "$rc" "a pull_request trigger is reported" "$out"

d=$(fresh); sed -i 's|^          path: site$|          path: .|' "$d/pages.yml"
out=$(check_workflows "$d"); rc=$?
expect_status 1 "$rc" "uploading the whole repository is reported" "$out"
expect_in "doesn't upload path: site" "$out" "path message"

d=$(fresh); printf '      - run: make check\n' >> "$d/pages.yml"
out=$(check_workflows "$d"); rc=$?
expect_status 1 "$rc" "a command in the Pages workflow is reported" "$out"
expect_in "runs commands" "$out" "run message"

d=$(fresh); sed -i 's|^  workflow_dispatch:|  push:\n  workflow_dispatch:|' "$d/server.yml"
out=$(check_workflows "$d"); rc=$?
expect_status 1 "$rc" "another workflow starting on push is reported" "$out"
expect_in "server.yml isn't manual only" "$out" "manual-only message"
done_testing
