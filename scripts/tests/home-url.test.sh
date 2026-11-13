#!/usr/bin/env bash
# The project's home address is the same everywhere it is written: the README's "The project's
# page" line, and the landing page's og:url and canonical link (site/index.html). Also checks, on
# scratch copies, that a missing or different address is reported.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

# check_home <root>: prints each problem, exits 1 if any.
check_home() {
  local root=$1 bad=0 want og canon
  want=$(grep -oE "^The project's page: \*\*<https://[^>]+>\*\*" "$root/README.md" 2>/dev/null \
    | sed -E 's/.*<([^>]+)>.*/\1/' | head -n1)
  if [ -z "$want" ]; then
    echo "README.md has no \"The project's page: **<https://…>**\" line"; return 1
  fi
  og=$(tr '\n' ' ' < "$root/site/index.html" | grep -oiE '<meta property="og:url" content="[^"]*"' \
    | sed -E 's/.*content="([^"]*)"/\1/' | head -n1)
  canon=$(tr '\n' ' ' < "$root/site/index.html" | grep -oiE '<link rel="canonical" href="[^"]*"' \
    | sed -E 's/.*href="([^"]*)"/\1/' | head -n1)
  [ "$og" = "$want" ] || { echo "site/index.html og:url is '${og:-none}', README says '$want'"; bad=1; }
  [ "$canon" = "$want" ] || { echo "site/index.html canonical is '${canon:-none}', README says '$want'"; bad=1; }
  return $bad
}

out=$(check_home "$REPO"); rc=$?
expect_status 0 "$rc" "the README and the landing page name the same home address" "$out"

d="$SCRATCH/repo"
mkdir -p "$d/site"
page() {
  printf '<meta property="og:url" content="%s">\n<link rel="canonical" href="%s">\n' "$1" "$2" > "$d/site/index.html"
}
printf "The project's page: **<https://example.org/>** (source).\n" > "$d/README.md"
page https://example.org/ https://example.org/
out=$(check_home "$d"); rc=$?
expect_status 0 "$rc" "matching addresses pass" "$out"

page https://example.org/ https://old.example.net/x/
out=$(check_home "$d"); rc=$?
expect_status 1 "$rc" "a different canonical address is reported" "$out"
expect_in "old.example.net" "$out" "the report names the other address"

page https://old.example.net/x/ https://example.org/
out=$(check_home "$d"); rc=$?
expect_status 1 "$rc" "a different og:url is reported" "$out"
expect_in "og:url" "$out" "the report names the tag"

printf '<title>x</title>\n' > "$d/site/index.html"
out=$(check_home "$d"); rc=$?
expect_status 1 "$rc" "a page without the tags is reported" "$out"

page https://example.org/ https://example.org/
printf 'No home line here.\n' > "$d/README.md"
out=$(check_home "$d"); rc=$?
expect_status 1 "$rc" "a README without the home line is reported" "$out"
done_testing
