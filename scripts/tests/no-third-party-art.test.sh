#!/usr/bin/env bash
# The apps contact only the Moonrise server (ADR 0013): no address of TMDB's, and no image loaded
# from a candidate's `poster` field, in the Apple and Android sources. Also checks that a use of one
# is reported, on a scratch file.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

# find_uses <root>...: prints each line that loads art from a third party, exits 1 if any.
find_uses() {
  ! grep -rnE --include='*.swift' --include='*.kt' --exclude-dir=MoonriseTests --exclude-dir=test \
    -e 'image\.tmdb\.org|themoviedb\.org|tmdb\.org/t/p' \
    -e 'AsyncImage\(url: *[a-zA-Z]+\.poster\b' \
    "$@"
}

out=$(find_uses "$REPO/clients/apple" "$REPO/clients/android"); rc=$?
expect_status 0 "$rc" "no app loads art from a third party" "$out"

printf 'let u = "https://image.tmdb.org/t/p/w342/a.jpg"\n' > "$SCRATCH/A.kt"
out=$(find_uses "$SCRATCH"); rc=$?
expect_status 1 "$rc" "a TMDB address is reported" "$out"
rm "$SCRATCH/A.kt"
printf 'AsyncImage(url: c.poster.flatMap(URL.init(string:)))\n' > "$SCRATCH/B.swift"
out=$(find_uses "$SCRATCH"); rc=$?
expect_status 1 "$rc" "a candidate's poster is reported" "$out"
done_testing
