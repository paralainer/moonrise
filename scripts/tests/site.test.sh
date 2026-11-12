#!/usr/bin/env bash
# The landing page (site/) stays self-contained and light: scripts only from its own .js
# files (no inline code, no inline handlers), nothing loaded from another host (fonts, styles,
# images, CDNs, trackers), no network calls or other hosts in its scripts, every local file it
# names exists (stylesheets' url()s and subtitle tracks too), self-hosted fonts carry their
# licence, every image has its size and a text alternative, the credits page links every
# source CREDITS.md names, and the whole site stays under 10 MB, the videos included. Also
# checks that a page breaking each rule is reported, on scratch copies.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

MAX_KB=${SITE_MAX_KB:-10240}

# check_site <dir>: prints each problem, exits 1 if any.
check_site() {
  local dir=$1 rc=0 f ref kb
  for f in "$dir"/*.html; do
    [ -e "$f" ] || continue
    # A script tag is only <script src="local.js" defer></script>: no inline code, no handlers.
    if tr '\n' ' ' < "$f" | grep -oiE '<script[^>]*>[^<]*</script>' | grep -viE '^<script src="[a-z0-9_-]+\.js"( defer)?></script>$'; then
      echo "$f: has an inline or outside script"; rc=1
    fi
    if grep -qiE '<script[^>]*/>|javascript:|[[:space:]]on[a-z]+=' "$f"; then
      echo "$f: has inline script code"; rc=1
    fi
    # Anything the page loads (src, srcset, stylesheets, icons) is local. Links (<a href>) may go out.
    if grep -oiE '<(img|source|link|iframe|video|audio|embed|object)[^>]*>' "$f" \
        | grep -iE '(src|srcset|href|data)="[^"]*(https?:)?//' \
        | grep -viE '<link[^>]*rel="(canonical|me)"'; then
      echo "$f: loads something from another host"; rc=1
    fi
    # Every image has a size (no layout shift) and an alt.
    while IFS= read -r tag; do
      case "$tag" in *' width="'*) ;; *) echo "$f: image without width: $tag"; rc=1 ;; esac
      case "$tag" in *' height="'*) ;; *) echo "$f: image without height: $tag"; rc=1 ;; esac
      case "$tag" in *' alt="'*) ;; *) echo "$f: image without alt: $tag"; rc=1 ;; esac
    done < <(tr '\n' ' ' < "$f" | grep -oiE '<img[^>]*>')
    # Local files it names exist.
    while IFS= read -r ref; do
      ref=${ref%%#*}; ref=${ref%%\?*}
      [ -n "$ref" ] || continue
      [ -e "$dir/$ref" ] || { echo "$f: missing $ref"; rc=1; }
    done < <(grep -oiE '(src|href)="[^"]*"' "$f" | sed -E 's/^[a-z]+="//I; s/"$//' | grep -vE '^(https?:|mailto:|#)';
             grep -oiE 'srcset="[^"]*"' "$f" | sed -E 's/^srcset="//I; s/"$//' | tr ',' '\n' | awk '{print $1}')
  done
  for f in "$dir"/*.js; do
    [ -e "$f" ] || continue
    if grep -nE 'https?:|//[a-z0-9.-]+\.[a-z]{2,}/|fetch\(|XMLHttpRequest|WebSocket|sendBeacon|EventSource|importScripts|import\(' "$f"; then
      echo "$f: talks to the network or names another host"; rc=1
    fi
  done
  for f in "$dir"/*.css; do
    [ -e "$f" ] || continue
    if grep -qiE '@import|url\([^)]*//' "$f"; then
      echo "$f: loads a font or a file from another host"; rc=1
    fi
    # A font or image the stylesheet names (url(...), as a self-hosted @font-face does) exists.
    while IFS= read -r ref; do
      ref=${ref%%#*}; ref=${ref%%\?*}
      case "$ref" in ''|data:*) continue ;; esac
      [ -e "$dir/$ref" ] || { echo "$f: missing $ref"; rc=1; }
    done < <(grep -oE 'url\([^)]*\)' "$f" | sed -E 's/^url\(["'"'"']?//; s/["'"'"']?\)$//')
  done
  # Fonts are self-hosted WOFF2 with their licence beside them.
  for f in "$dir"/fonts/*.woff2; do
    [ -e "$f" ] || continue
    [ -e "$dir/fonts/OFL.txt" ] || { echo "$f: no fonts/OFL.txt beside it"; rc=1; }
  done
  # The credits page names every source CREDITS.md names (the two say the same).
  if [ -e "$dir/CREDITS.md" ]; then
    while IFS= read -r ref; do
      grep -qF "href=\"$ref\"" "$dir/credits.html" 2>/dev/null || { echo "$dir/credits.html: doesn't link $ref (from CREDITS.md)"; rc=1; }
    done < <(grep -oE 'https?://[^][:space:]|()<>]+' "$dir/CREDITS.md" | sort -u)
  fi
  kb=$(du -sk --exclude=README.md "$dir" | cut -f1)
  if [ "$kb" -gt "$MAX_KB" ]; then echo "$dir: ${kb} KB, more than ${MAX_KB} KB"; rc=1; fi
  return $rc
}

out=$(check_site "$REPO/site"); rc=$?
expect_status 0 "$rc" "site/ is self-contained and light" "$out"

# Each rule reports a page that breaks it.
bad() {  # bad <description> <html body>
  local d="$SCRATCH/site-$RANDOM"
  mkdir -p "$d/img"; : > "$d/img/a.png"
  printf '<!doctype html><html><body>%s</body></html>\n' "$2" > "$d/index.html"
  out=$(check_site "$d"); rc=$?
  expect_status 1 "$rc" "$1 is reported" "$out"
}
bad "an inline script" '<script>alert(1)</script>'
bad "a script from another host" '<script src="https://cdn.example/x.js"></script>'
bad "a missing script file" '<script src="nope.js" defer></script>'
bad "an inline handler" '<a href="#" onclick="x()">x</a>'
bad "an image from another host" '<img src="https://cdn.example/a.png" width="1" height="1" alt="">'
bad "a stylesheet from another host" '<link rel="stylesheet" href="//fonts.example/css">'
bad "an image without a size" '<img src="img/a.png" alt="">'
bad "an image without alt" '<img src="img/a.png" width="1" height="1">'
bad "a missing file" '<img src="img/b.png" width="1" height="1" alt="">'
bad "a missing srcset file" '<picture><source srcset="img/a.png 1x, img/c.webp 2x"><img src="img/a.png" width="1" height="1" alt=""></picture>'
d="$SCRATCH/site-css"; mkdir -p "$d"; printf '<!doctype html><p>x</p>\n' > "$d/index.html"
printf '@import url("https://fonts.example/x.css");\n' > "$d/style.css"
out=$(check_site "$d"); rc=$?
expect_status 1 "$rc" "a stylesheet importing from another host is reported" "$out"
d="$SCRATCH/site-js"; mkdir -p "$d"; printf '<!doctype html><script src="a.js" defer></script>\n' > "$d/index.html"
printf 'fetch("/x")\n' > "$d/a.js"
out=$(check_site "$d"); rc=$?
expect_status 1 "$rc" "a script that calls the network is reported" "$out"
d="$SCRATCH/site-credits"; mkdir -p "$d"; printf '<!doctype html><p>x</p>\n' > "$d/index.html"
printf 'Film: https://example.org/film.mov\n' > "$d/CREDITS.md"
printf '<!doctype html><a href="https://example.org/other.mov">x</a>\n' > "$d/credits.html"
out=$(check_site "$d"); rc=$?
expect_status 1 "$rc" "a credits page missing a source from CREDITS.md is reported" "$out"
d="$SCRATCH/site-font"; mkdir -p "$d"; printf '<!doctype html><link rel="stylesheet" href="style.css">\n' > "$d/index.html"
printf '@font-face { font-family: X; src: url("fonts/x.woff2") format("woff2"); }\n' > "$d/style.css"
out=$(check_site "$d"); rc=$?
expect_status 1 "$rc" "a stylesheet naming a missing font file is reported" "$out"
mkdir -p "$d/fonts"; : > "$d/fonts/x.woff2"
out=$(check_site "$d"); rc=$?
expect_status 1 "$rc" "a font without its licence beside it is reported" "$out"
: > "$d/fonts/OFL.txt"
out=$(check_site "$d"); rc=$?
expect_status 0 "$rc" "a self-hosted font with its licence passes" "$out"
d="$SCRATCH/site-track"; mkdir -p "$d"
printf '<!doctype html><video width="2" height="1"><track kind="subtitles" src="film/a.vtt"></video>\n' > "$d/index.html"
out=$(check_site "$d"); rc=$?
expect_status 1 "$rc" "a missing subtitle track is reported" "$out"
# A good page passes, with links out and a local script.
d="$SCRATCH/site-good"; mkdir -p "$d/img"; : > "$d/img/a.png"
printf '<!doctype html><a href="https://example.org/">x</a><img src="img/a.png" width="1" height="1" alt=""><script src="a.js" defer></script>\n' > "$d/index.html"
printf 'document.body.className = "js";\n' > "$d/a.js"
out=$(check_site "$d"); rc=$?
expect_status 0 "$rc" "a page with only local files and links out passes" "$out"
done_testing
