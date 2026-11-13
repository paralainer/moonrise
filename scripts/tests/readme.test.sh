#!/usr/bin/env bash
# The README states the server's real version (the one in server/Cargo.toml), has no leftover
# placeholders (a clone URL to fill in, a list of screenshots still to add), and every image it
# shows is in the repository. Also checks, on scratch copies, that each problem is reported.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

# check_readme <root>: prints each problem, exits 1 if any.
check_readme() {
  local root=$1 bad=0 want got img
  want=$(sed -nE '/^\[package\]/,/^\[/ s/^version *= *"([^"]+)".*/\1/p' "$root/server/Cargo.toml" | head -n1)
  got=$(grep -oE 'The server is at version [0-9]+\.[0-9]+\.[0-9]+' "$root/README.md" | grep -oE '[0-9]+\.[0-9]+\.[0-9]+$')
  if [ -z "$want" ] || [ "$got" != "$want" ]; then
    echo "README says server version '${got:-none}', server/Cargo.toml has '${want:-none}'"; bad=1
  fi
  if grep -nF "<this repository's URL>" "$root/README.md"; then echo "a clone URL placeholder is left"; bad=1; fi
  if grep -niE 'screenshots to add' "$root/README.md"; then echo "a screenshot to-do list is left"; bad=1; fi
  while read -r img; do
    [ -f "$root/$img" ] || { echo "README shows $img, which isn't there"; bad=1; }
  done < <(grep -oE '<img [^>]*src="[^":]+"' "$root/README.md" | sed -E 's/.*src="([^"]+)"/\1/')
  return $bad
}

out=$(check_readme "$REPO"); rc=$?
expect_status 0 "$rc" "the README's version, clone URL and screenshots are current" "$out"

# A scratch copy with each problem in turn.
d="$SCRATCH/repo"
mkdir -p "$d/server" "$d/site/shots"
: > "$d/site/shots/a.webp"
printf '[package]\nname = "x"\nversion = "1.2.3"\n\n[dependencies]\nfoo = { version = "9.9.9" }\n' > "$d/server/Cargo.toml"
good='The server is at version 1.2.3 (notes).\n\n<img src="site/shots/a.webp" width="10">\n\ngit clone https://example.org/x.git\n'
printf "$good" > "$d/README.md"
out=$(check_readme "$d"); rc=$?
expect_status 0 "$rc" "a current README passes" "$out"

printf 'The server is at version 1.2.2 (notes).\n' > "$d/README.md"
out=$(check_readme "$d"); rc=$?
expect_status 1 "$rc" "a stale version is reported" "$out"
expect_in "1.2.3" "$out" "the report names the Cargo.toml version"

printf "$good"'git clone <this repository'"'"'s URL> x\n' > "$d/README.md"
out=$(check_readme "$d"); rc=$?
expect_status 1 "$rc" "a clone URL placeholder is reported" "$out"

printf "$good"'<!-- Screenshots to add before publishing -->\n' > "$d/README.md"
out=$(check_readme "$d"); rc=$?
expect_status 1 "$rc" "a screenshot to-do list is reported" "$out"

printf "$good"'<img src="site/shots/gone.webp">\n' > "$d/README.md"
out=$(check_readme "$d"); rc=$?
expect_status 1 "$rc" "a missing screenshot is reported" "$out"
done_testing
