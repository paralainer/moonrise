#!/usr/bin/env bash
# The repository's links and image names name its real account: no `OWNER` placeholder is left in a
# GitHub, GHCR or GitHub Pages address in the tracked files. Also checks that one is reported, in a
# scratch repository.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

# find_placeholders <repo>: prints each tracked line with a placeholder address, exits 1 if any.
find_placeholders() {
  ! git -C "$1" grep -nE \
    -e '(github\.com|ghcr\.io|githubusercontent\.com)/OWNER\b' \
    -e '\bOWNER\.github\.io' \
    -- .
}

out=$(find_placeholders "$REPO"); rc=$?
expect_status 0 "$rc" "no OWNER placeholder address is left" "$out"

d="$SCRATCH/repo"
mkdir -p "$d/docs" && git -C "$d" init -q
printf 'image: ghcr.io/paralainer/moonrise-server:latest\nDOWNLOAD_OWNER and the owner of the account\n' > "$d/docs/ok.md"
git -C "$d" add -A
out=$(find_placeholders "$d"); rc=$?
expect_status 0 "$rc" "real addresses and the word OWNER elsewhere pass" "$out"

# The placeholder is spliced in from a variable, so this file never contains the pattern it looks for.
O=OWNER
for line in "image: ghcr.io/$O/moonrise-server:latest" \
            "<a href=\"https://github.com/$O/moonrise\">Source</a>" \
            "https://raw.githubusercontent.com/$O/moonrise/main/x.png" \
            "https://$O.github.io/moonrise/"; do
  printf '%s\n' "$line" > "$d/docs/bad.md"
  git -C "$d" add -A
  out=$(find_placeholders "$d"); rc=$?
  expect_status 1 "$rc" "a placeholder is reported: $line" "$out"
  expect_in "docs/bad.md:1:" "$out" "the report names the file and line"
done
done_testing
