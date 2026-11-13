#!/usr/bin/env bash
# Every illustration the apps ship (a JPEG in the Apple asset catalogs or an Android res folder)
# has its origin written in assets/brand/README.md. Also checks, on a scratch copy, that an
# uncredited one is reported.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

# uncredited <root>: prints each app illustration assets/brand/README.md doesn't name, exits 1 if any.
uncredited() {
  local root=$1 bad=0 f name
  while read -r f; do
    case "$f" in
      *.imageset/*) name=$(basename "$(dirname "$f")" .imageset) ;;
      *) name=$(basename "$f" .jpg) ;;
    esac
    grep -qF -- "$name" "$root/assets/brand/README.md" || { echo "no origin for $f"; bad=1; }
  done < <(cd "$root" && find clients/apple clients/android -name '*.jpg' \
    \( -path '*.xcassets/*' -o -path '*/src/*/res/*' \) 2>/dev/null | sort)
  return $bad
}

out=$(uncredited "$REPO"); rc=$?
expect_status 0 "$rc" "every illustration in the apps has its origin written down" "$out"

d="$SCRATCH/repo"
mkdir -p "$d/assets/brand" "$d/clients/apple/A.xcassets/Hero.imageset" "$d/clients/android/phone/src/main/res/drawable-nodpi"
: > "$d/clients/apple/A.xcassets/Hero.imageset/Hero.jpg"
: > "$d/clients/android/phone/src/main/res/drawable-nodpi/hero_art.jpg"
printf '| `Hero` | `hero_art.jpg` |\n' > "$d/assets/brand/README.md"
out=$(uncredited "$d"); rc=$?
expect_status 0 "$rc" "credited illustrations pass" "$out"
: > "$d/clients/android/phone/src/main/res/drawable-nodpi/stray.jpg"
out=$(uncredited "$d"); rc=$?
expect_status 1 "$rc" "an uncredited illustration is reported" "$out"
expect_in "stray.jpg" "$out" "the report names it"
done_testing
