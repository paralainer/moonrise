#!/usr/bin/env bash
# No tracked file names an issue of the project's private tracker (`VP-` and a number): those
# issues aren't public, so such a reference means nothing to a reader; say what it meant in words.
# Also checks, on a scratch repository, that a reference is reported.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

PATTERN='\bVP-[0-9]+'

# find_refs <dir>: prints each tracked text line with a reference, exits 1 if any.
find_refs() {
  ! git -C "$1" grep -InE "$PATTERN"
}

out=$(find_refs "$REPO"); rc=$?
expect_status 0 "$rc" "no tracked file names a private ticket" "$out"

# The reference is put together at run time, so this file has none to report.
R="$SCRATCH/refs"; mkdir -p "$R"; git -C "$R" init -q
printf 'fine: VP9, HEVC and AV1\n' > "$R/codecs.txt"
git -C "$R" add codecs.txt
out=$(find_refs "$R"); rc=$?
expect_status 0 "$rc" "a codec name isn't a ticket" "$out"

printf '// Fixed in %s%s-%s.\n' V P 123 > "$R/a.kt"
out=$(find_refs "$R"); rc=$?
expect_status 0 "$rc" "an untracked file isn't checked" "$out"
git -C "$R" add a.kt
out=$(find_refs "$R"); rc=$?
expect_status 1 "$rc" "a ticket reference is reported" "$out"
expect_in "a.kt:1:" "$out" "the report names the file and line"
done_testing
