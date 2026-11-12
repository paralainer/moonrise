#!/usr/bin/env bash
# Runs the tests of the repository's shell scripts (scripts/tests/*.test.sh): offline, in scratch
# folders, with ssh and the build Mac stubbed. Exits non-zero when any fails.
cd "$(dirname "${BASH_SOURCE[0]}")" || exit 1
rc=0
for t in ./*.test.sh; do
  bash "$t" || rc=1
done
exit $rc
