#!/usr/bin/env bash
# The fastlane lanes' plain-Ruby rules (clients/apple/fastlane/release_helpers.rb): the submit
# lane's confirm guard (true or "yes" only) and manual release (set, read back, refused when it
# doesn't stick), and the Fastfile's Ruby syntax. Needs ruby (the build Mac has it with fastlane),
# else a local ruby Docker image ($RUBY_IMAGE); without either it is skipped, saying so.
cd "$(dirname "${BASH_SOURCE[0]}")/../../clients/apple/fastlane" || exit 1
RUBY_IMAGE=${RUBY_IMAGE:-ruby:3.3-slim}
if command -v ruby >/dev/null 2>&1; then
  RUBY=(ruby)
elif command -v docker >/dev/null 2>&1 && docker image inspect "$RUBY_IMAGE" >/dev/null 2>&1; then
  RUBY=(docker run --rm -v "$PWD:/f:ro" -w /f "$RUBY_IMAGE" ruby)
else
  echo "fastlane-helpers.test.sh: skipped (no ruby, and no local $RUBY_IMAGE image; docker pull it once, or run this on the build Mac)"
  exit 0
fi
rc=0
"${RUBY[@]}" -c Fastfile >/dev/null || { echo "FAIL fastlane-helpers.test.sh: Fastfile has a Ruby syntax error"; rc=1; }
"${RUBY[@]}" -c release_helpers.rb >/dev/null || { echo "FAIL fastlane-helpers.test.sh: release_helpers.rb has a Ruby syntax error"; rc=1; }
"${RUBY[@]}" test/release_helpers_test.rb || rc=1
exit $rc
