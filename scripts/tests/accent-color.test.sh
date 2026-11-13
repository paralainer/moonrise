#!/usr/bin/env bash
# The app's white accent (AccentColor) is for iPhone and iPad only. On macOS AppKit draws the
# hovered menu row in the app's accent with white text, so a white one left a blank white box;
# on tvOS it turned focused button text white on the white platter. Both use the system accent.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

YML="$REPO/clients/apple/project.yml"
setting() { grep -E "^ *\"?ASSETCATALOG_COMPILER_GLOBAL_ACCENT_COLOR_NAME$1\"?:" "$YML" | head -1 | sed 's/^[^:]*: *//'; }

[ "$(setting '')" = AccentColor ] && ok || fail "the app's accent is AccentColor" "$(setting '')"
for sdk in macosx appletv; do
  got=$(setting "\\[sdk=$sdk\\*\\]")
  [ "$got" = '""' ] && ok || fail "$sdk builds use the system accent (an empty accent name)" "${got:-<no setting>}"
done

done_testing
