#!/usr/bin/env bash
# The support page is named once in .github/FUNDING.yml (buy_me_a_coffee: <handle>), and the
# README and the landing page (site/index.html) link that same page and no other Buy Me a Coffee
# page. Also checks, on scratch copies, that a missing or different link is reported.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

# check_support <root>: prints each problem, exits 1 if any.
check_support() {
  local root=$1 bad=0 handle want f other
  handle=$(sed -nE 's/^buy_me_a_coffee:[[:space:]]*([A-Za-z0-9_-]+)[[:space:]]*$/\1/p' "$root/.github/FUNDING.yml" 2>/dev/null | head -n1)
  if [ -z "$handle" ]; then
    echo ".github/FUNDING.yml names no buy_me_a_coffee handle"; return 1
  fi
  want="https://buymeacoffee.com/$handle"
  for f in README.md site/index.html; do
    grep -qF "$want" "$root/$f" 2>/dev/null || { echo "$f doesn't link $want"; bad=1; }
    other=$(grep -oE 'https?://(www\.)?buymeacoffee\.com/[A-Za-z0-9_-]*' "$root/$f" 2>/dev/null | grep -vxF "$want" | sort -u)
    [ -z "$other" ] || { echo "$f links another support page: $other"; bad=1; }
  done
  return $bad
}

out=$(check_support "$REPO"); rc=$?
expect_status 0 "$rc" "FUNDING.yml, the README and the landing page name the same support page" "$out"

d="$SCRATCH/repo"
mkdir -p "$d/.github" "$d/site"
printf 'buy_me_a_coffee: someone\n' > "$d/.github/FUNDING.yml"
printf 'Support: [page](https://buymeacoffee.com/someone).\n' > "$d/README.md"
printf '<a href="https://buymeacoffee.com/someone">x</a>\n' > "$d/site/index.html"
out=$(check_support "$d"); rc=$?
expect_status 0 "$rc" "matching links pass" "$out"

printf '<p>no link</p>\n' > "$d/site/index.html"
out=$(check_support "$d"); rc=$?
expect_status 1 "$rc" "a page without the link is reported" "$out"
expect_in "site/index.html" "$out" "the report names the page"

printf '<a href="https://buymeacoffee.com/someone">x</a>\n' > "$d/site/index.html"
printf 'Support: [page](https://buymeacoffee.com/someone-else).\n' > "$d/README.md"
out=$(check_support "$d"); rc=$?
expect_status 1 "$rc" "a link to a different page is reported" "$out"
expect_in "someone-else" "$out" "the report names the other page"

printf 'Support: [page](https://buymeacoffee.com/someone).\n' > "$d/README.md"
printf '# buy_me_a_coffee: someone\n' > "$d/.github/FUNDING.yml"
out=$(check_support "$d"); rc=$?
expect_status 1 "$rc" "a commented-out handle is reported" "$out"
done_testing
