#!/usr/bin/env bash
# Make the public copy of Moonrise: the committed tree (HEAD) in a new directory with a fresh,
# one-commit history, then scan it for secrets (gitleaks, in Docker) and for private details
# (the patterns below, plus your own: scripts/local/private-patterns.txt, if present, and your
# user file, ~/.config/moonrise/private-patterns.txt or the file named by MOONRISE_PRIVATE_PATTERNS).
# Exits non-zero on any finding; the copy stays for inspection.
#
# Without the user file at its usual place, the run says so (in the private-details scan and on
# the closing lines) and checks only the generic patterns. A file named by MOONRISE_PRIVATE_PATTERNS
# must be there and readable: otherwise the run stops at once with exit status 4, as a typo there
# would drop your patterns without a word.
#
#   scripts/export-public.sh <target dir> [--author "Name <email>"]
#
# This repository's history is never touched and never copied. Git-ignored files (signing
# identity, build machine, credential hooks, private notes) aren't in HEAD, so they never reach
# the copy.
#
# Exit status: 0 clean, 1 findings, 2 target directory not empty, 3 no findings but the built-in
# keys weren't fully checked, 4 MOONRISE_PRIVATE_PATTERNS names a file that isn't there or can't
# be read. On this installation (builtin-keys.sh present) a check that can't be
# completed is a failed run with its own closing line, and no option overrides it: a warning above
# "ready to push" is too easy to miss. That covers a helper that fails or can't be run, one that
# doesn't give a value for every MOONRISE_BUILTIN_ name in server/src/admin/config.rs (which includes
# BUILTIN_SKIP), and one that prints anything but plain MOONRISE_BUILTIN_NAME=value lines.
# Only a clone with no helper at all (a contributor's) may pass; it skips the check and says so,
# also on the closing lines. The helper's stderr is discarded, as it could contain a value; only
# a short reason, key names and file names are printed, and tracing (bash -x) is off for as long
# as the values are in use.
#
# No scan prints the text of a line it matched, since the rest of the line could hold a key: the
# private-details scan gives file:line:column, the pattern's label and the match's length, with
# its first 2 characters only when those are written out in the pattern itself; the former-name
# scan gives the place and the word, or the path, with every character but the name's own masked;
# the key scan gives file names only.
#
# Every scan runs to the end, whatever an earlier one found, so the summary and the exit status
# cover all of them. Long lists are cut with sed -n, which reads all its input: head would close
# the pipe early, and under pipefail the writer's SIGPIPE would end the script (exit 141).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
case "${1:-}" in
  -h|--help) awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }' "$0"; exit 0 ;;
esac
OUT=${1:?usage: $0 <target dir> [--author "Name <email>"]}
AUTHOR="Moonrise contributors <moonrise@users.noreply.github.com>"
[ "${2:-}" = --author ] && AUTHOR=${3:?--author needs a value}
# Your user patterns file (see "private details" below). Never printed, only its path.
USER_PATTERNS=${MOONRISE_PRIVATE_PATTERNS:-${XDG_CONFIG_HOME:-$HOME/.config}/moonrise/private-patterns.txt}
if [ -n "${MOONRISE_PRIVATE_PATTERNS:-}" ] && ! { [ -f "$USER_PATTERNS" ] && [ -r "$USER_PATTERNS" ]; }; then
  echo "MOONRISE_PRIVATE_PATTERNS names $USER_PATTERNS, which isn't there or can't be read." >&2
  echo "Fix the path, or unset it to use ~/.config/moonrise/private-patterns.txt. Nothing was exported." >&2
  exit 4
fi
USER_PATTERNS_MISSING=0
if [ -e "$OUT" ] && [ -n "$(ls -A "$OUT" 2>/dev/null)" ]; then
  echo "$OUT exists and isn't empty" >&2; exit 2
fi
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"

# Paths of this repository that never go public, even though they are tracked.
EXCLUDE=(
  .playwright-mcp
)

echo "== exporting $(git -C "$ROOT" rev-parse --short HEAD) to $OUT"
git -C "$ROOT" archive --format=tar HEAD | tar -x -C "$OUT"
for p in "${EXCLUDE[@]}"; do rm -rf "${OUT:?}/$p"; done
NAME=${AUTHOR% <*}; EMAIL=${AUTHOR##*<}; EMAIL=${EMAIL%>}
(cd "$OUT" && git init -q -b main && git add -A \
  && git -c user.name="$NAME" -c user.email="$EMAIL" -c commit.gpgsign=false commit -q -m "Moonrise: first public release" \
  && echo "   $(git ls-files | wc -l) files, $(du -sh --exclude=.git . | cut -f1), one commit")

FAIL=0

echo "== gitleaks (secrets)"
if docker run --rm -v "$OUT:/repo:ro" zricethezav/gitleaks:v8.21.2 git /repo --no-banner --redact \
     --report-path /dev/null ${GITLEAKS_CONFIG:+--config "/repo/$GITLEAKS_CONFIG"} >/tmp/moonrise-gitleaks.$$ 2>&1; then
  echo "   no leaks"
else
  sed 's/^/   /' /tmp/moonrise-gitleaks.$$ | tail -40
  FAIL=1
fi
rm -f /tmp/moonrise-gitleaks.$$

echo "== private details"
# Extended regexes, case-insensitive. Generic examples (192.168.1.x, example.com) are fine. These
# are kinds of private detail any installation has; a contributor's own addresses, names, device
# ids and accounts go in their own patterns file (below), never in this tracked list.
PATTERNS=(
  '[a-z0-9._%+-]+@gmail\.com'          # personal mail addresses
  'AuthKey_[A-Z0-9]{10}\.p8'           # App Store Connect key files
  'tail[0-9a-f]{4,}\.ts\.net'          # tailnet names
  'op://|OP_SERVICE_ACCOUNT'            # the password manager
  '/tmp/claude-[0-9]+|\.claude/'        # agent scratch paths
)
# What a hit is called in the report: the pattern's comment above, else its place in the list.
LABELS=()
while IFS= read -r l; do
  case $l in *"'"*) ;; *) continue ;; esac
  c=$(printf '%s\n' "$l" | sed -E "s/^[[:space:]]*'[^']*'[[:space:]]*//; s/^#[[:space:]]*//")
  LABELS+=("${c:-pattern $(( ${#LABELS[@]} + 1 ))}")
done < <(sed -n '/^PATTERNS=(/,/^)/p' "${BASH_SOURCE[0]}")
if [ "${#LABELS[@]}" != "${#PATTERNS[@]}" ]; then
  LABELS=(); for i in "${!PATTERNS[@]}"; do LABELS+=("pattern $((i + 1))"); done
fi
# Your own patterns (one extended regex a line, # for comments): the git-ignored
# scripts/local/private-patterns.txt of this checkout, and your user file, which every clone of
# yours reads: $MOONRISE_PRIVATE_PATTERNS, else ~/.config/moonrise/private-patterns.txt.
add_patterns() {   # add_patterns <file> <label>
  local n=0 l
  [ -f "$1" ] || return 0
  while IFS= read -r l || [ -n "$l" ]; do
    n=$((n + 1))
    [ -n "$l" ] && [ "${l#\#}" = "$l" ] && PATTERNS+=("$l") && LABELS+=("$2 line $n")
  done < "$1"
}
add_patterns "$ROOT/scripts/local/private-patterns.txt" private-patterns.txt
if [ -f "$USER_PATTERNS" ] && [ -r "$USER_PATTERNS" ]; then
  add_patterns "$USER_PATTERNS" "your private-patterns.txt"
else
  USER_PATTERNS_MISSING=1
  echo "   no personal patterns file at $USER_PATTERNS: only the generic patterns of scripts/export-public.sh"
  echo "   are checked (and this clone's scripts/local/private-patterns.txt, if present). To add yours,"
  echo "   create it with one extended regex a line (# for comments); see CONTRIBUTING.md."
fi
# Each hit is reported by place and label and the match's length. The line is never printed: the
# rest of it could hold anything, a key included. Of the match itself, the first 2 characters are
# shown only when they are the start of one of the pattern's alternatives as written (after a
# leading ^ or \b, with \. read as .), so they tell nothing the pattern doesn't: a match that
# starts with a character class, like the mail addresses', could start with a glued-on key, and
# shows none.
# This script lists the patterns themselves: it is the one file allowed to contain them.
# An extra pattern's trailing comment starts at a # with whitespace before it; a # inside it stays.
HITS=""
for i in "${!PATTERNS[@]}"; do
  P=$(printf '%s\n' "${PATTERNS[$i]}" | sed -E 's/[[:space:]]+#.*$//')
  [ -n "$P" ] || continue
  H=$(cd "$OUT" && { git grep -z -nIiE --column -o -e "$P" -- . ':!scripts/export-public.sh' || true; } \
    | LC_ALL=C tr '\000' '\001' | PAT=$P LABEL=${LABELS[$i]} LC_ALL=C awk -F'\001' '
      BEGIN { n = split(ENVIRON["PAT"], alt, "|")
        for (a = 1; a <= n; a++) {
          s = alt[a]; sub(/^\^/, "", s); sub(/^\\b/, "", s); lit = ""
          while (length(lit) < 2 && s != "") {
            c = substr(s, 1, 1); s = substr(s, 2)
            if (c == "\\") { c = substr(s, 1, 1); s = substr(s, 2); if (c !~ /^[.\/@-]$/) break }
            else if (c !~ /^[a-zA-Z0-9_@\/:-]$/) break
            if (s ~ /^[?*{]/) break
            lit = lit c
          }
          if (length(lit) == 2) start[tolower(lit)] = 1
        } }
      { m = substr($4, 1, 2); m = (tolower(m) in start) ? m "... " : ""
        printf "%s:%s:%s  %s  [%s%d chars]\n", $1, $2, $3, ENVIRON["LABEL"], m, length($4) }')
  [ -z "$H" ] || HITS+="$H"$'\n'
done
HITS=${HITS%$'\n'}
if [ -n "$HITS" ]; then
  echo "$HITS" | sed 's/^/   /' | sed -n '1,80p'
  echo "   ($(echo "$HITS" | wc -l) matches; each line gives file:line:column, what matched and the match's length, never the line)"
  FAIL=1
else
  echo "   none"
fi

echo "== the project's former name"
# The project was called something else before it became Moonrise; nothing in the copy may still
# use that name, in any case, in file contents or paths. The pattern is spelled with a bracket so
# this file doesn't contain the word itself; the only allowed match is an English adverb in the
# vendored SMB crate's comments.
OLD='r[e]el'
# A hit shows the name's own letters in place and every other character of the word as *, so a
# word that was really a key (say) isn't printed. A path with the name in it is masked the same
# way except for its /, as a file name could be a key too: git ls-files | grep -i in the copy
# lists them in full.
MASK='function mask(w,  m, i, c) {
        if (!match(tolower(w), ENVIRON["OLD"])) return w
        m = ""
        for (i = 1; i <= length(w); i++) {
          c = substr(w, i, 1); m = m ((i >= RSTART && i < RSTART + RLENGTH) || c == "/" ? c : "*") }
        return m }'
HITS=$(cd "$OUT" && { git grep -z -nIi --column -o -E "[a-z0-9_]*$OLD[a-z0-9_]*" -- . || true; } \
  | LC_ALL=C tr '\000' '\001' | OLD=$OLD LC_ALL=C awk -F'\001' "$MASK"'
    tolower($4) ~ /^f[r]eely$/ { next }
    { printf "%s:%s:%s  [%s]\n", mask($1), $2, $3, mask($4) }')
# One path a line: a newline in a name becomes 0x01, which the mask turns into * like the rest.
PATHS=$(cd "$OUT" && git ls-files -z | LC_ALL=C tr '\n\000' '\001\n' | OLD=$OLD LC_ALL=C awk "$MASK"'
    tolower($0) ~ ENVIRON["OLD"] { printf "path: %s\n", mask($0) }')
if [ -n "$HITS$PATHS" ]; then
  printf '%s\n%s\n' "$HITS" "$PATHS" | sed '/^$/d' | cut -c1-200 | sed 's/^/   /' | sed -n '1,80p'
  echo "   ($(printf '%s\n%s\n' "$HITS" "$PATHS" | sed '/^$/d' | wc -l) matches)"
  FAIL=1
else
  echo "   none"
fi

echo "== text in images"
# Image files can carry text the pixels don't show (generator prompts, paths, camera data): PNG
# tEXt/iTXt/zTXt/eXIf chunks, JPEG comments and Exif/XMP. None may be in the copy.
META=$(cd "$OUT" && git ls-files -z '*.png' '*.jpg' '*.jpeg' | python3 -I -c '
import struct, sys
for f in sys.stdin.buffer.read().split(b"\0"):
    if not f: continue
    d = open(f, "rb").read(); found = set()
    if d[:8] == b"\x89PNG\r\n\x1a\n":
        i = 8
        while i + 8 <= len(d):
            n, t = struct.unpack(">I4s", d[i:i+8]); i += 12 + n
            if t in (b"tEXt", b"iTXt", b"zTXt", b"eXIf"): found.add(t.decode())
    elif d[:2] == b"\xff\xd8":
        i = 2
        while i + 4 <= len(d) and d[i] == 0xFF and d[i+1] not in (0xDA, 0xD9):
            if d[i+1] in (0xFE, 0xE1): found.add("COM" if d[i+1] == 0xFE else "APP1")
            i += 2 + struct.unpack(">H", d[i+2:i+4])[0]
    if found: print(f.decode(), ",".join(sorted(found)))
')
if [ -n "$META" ]; then
  echo "$META" | sed 's/^/   /' | sed -n '1,40p'
  echo "   ($(echo "$META" | wc -l) files; strip the metadata, keep the pixels)"
  FAIL=1
else
  echo "   none"
fi

echo "== the project's own keys"
# The built-in keys of this installation (server/deploy/local/builtin-keys.sh, git-ignored) must
# not be anywhere in the copy. Only names are printed, never the values. The values stay in shell
# variables and reach other programs through pipes from the shell's own printf, so this script
# doesn't write them to disk (a here-string could, when large) or show them in a process list.
# Tracing is off in this whole section, as it would print them.
case $- in *x*) KEYS_TRACE=1; set +x ;; *) KEYS_TRACE=0 ;; esac
KEYS_SH="$ROOT/server/deploy/local/builtin-keys.sh"
KEYS_SKIPPED=0
KEYS_UNCHECKED=0
not_checked() { echo "   NOT CHECKED: $1"; KEYS_UNCHECKED=1; }
# Keeps or (-v) drops lines of the text in $1, by bytes rather than by locale.
key_lines() { local text=$1; shift; printf '%s\n' "$text" | LC_ALL=C grep -a "$@" || true; }
key_names() { printf '%s\n' "$1" | LC_ALL=C sed -nE 's/^(MOONRISE_BUILTIN_[A-Z0-9_]+)=.*/\1/p' | LC_ALL=C sort -u; }
in_list() { case $'\n'"$2"$'\n' in *$'\n'"$1"$'\n'*) return 0 ;; *) return 1 ;; esac; }
if [ ! -e "$KEYS_SH" ] && [ ! -L "$KEYS_SH" ]; then
  echo "   skipped: no server/deploy/local/builtin-keys.sh here"
  KEYS_SKIPPED=1
elif [ ! -x "$KEYS_SH" ]; then
  not_checked "builtin-keys.sh is there but can't be run"
else
  RC=0
  # tr: bash would silently drop a NUL byte and join the two halves into a value that looks fine.
  KEYS_OUT=$("$KEYS_SH" 2>/dev/null | LC_ALL=C tr '\000' '\001') || RC=$?
  [ "$RC" = 0 ] || not_checked "builtin-keys.sh failed (exit status $RC)"
  # A value is searched for exactly as printed, so it has to be one this search can trust: at least
  # 8 characters (fewer would match too much), printable non-space ASCII, no quotes (the range
  # below is 0x21-0x7e without " and '). Anything else may not be the key as it's really written.
  KEY_LINE='^MOONRISE_BUILTIN_[A-Z0-9_]+='
  GOOD_LINE="$KEY_LINE[!#-&(-~]{8,}\$"
  GOOD=$(key_lines "$KEYS_OUT" -E "$GOOD_LINE")
  NAMED=$(key_lines "$KEYS_OUT" -E "$KEY_LINE")
  PRINTED=$(key_names "$NAMED")
  GOOD_NAMES=$(key_names "$GOOD")
  BAD_NAMES=$(key_names "$(key_lines "$NAMED" -vE "$GOOD_LINE")")
  # Any other line could be a piece of a value (the second line of one, say): counted, never shown.
  OTHER=0
  [ -z "$KEYS_OUT" ] || OTHER=$(key_lines "$KEYS_OUT" -cvE "$KEY_LINE")
  [ "$OTHER" = 0 ] || not_checked "builtin-keys.sh printed $OTHER line(s) that aren't MOONRISE_BUILTIN_NAME=value"
  for NAME in $BAD_NAMES; do
    not_checked "builtin-keys.sh's value for $NAME isn't one to search for (8+ characters of printable ASCII, no spaces or quotes)${BUILTIN_SKIP:+ (BUILTIN_SKIP is set)}"
  done
  # The keys the server knows, read from the copy so that a new one is demanded without a list here.
  # Every MOONRISE_BUILTIN_ name in that file counts as a key: name nothing else that way there.
  WANT=$(grep -ohE 'MOONRISE_BUILTIN_[A-Z0-9_]+' "$OUT/server/src/admin/config.rs" 2>/dev/null | LC_ALL=C sort -u || true)
  [ -n "$WANT" ] || not_checked "no key names found in server/src/admin/config.rs"
  for NAME in $WANT; do
    in_list "$NAME" "$PRINTED" \
      || not_checked "builtin-keys.sh gave no value for $NAME${BUILTIN_SKIP:+ (BUILTIN_SKIP is set)}"
  done
  if [ -n "$GOOD" ]; then
    VALUES=$(printf '%s\n' "$GOOD" | LC_ALL=C sed -E 's/^[^=]*=//')
    RC=0
    FOUND=$(printf '%s\n' "$VALUES" | (cd "$OUT" && git grep -lF -f - 2>/dev/null)) || RC=$?
    case $RC in
      0) echo "   a built-in key is in: $FOUND"; FAIL=1 ;;
      1) echo "   none in the copy (keys checked: $(key_lines "$GOOD" -c .))" ;;
      *) not_checked "the search of the copy failed (git grep exit status $RC)" ;;
    esac
    if [ "$RC" -le 1 ]; then
      for NAME in $GOOD_NAMES; do
        in_list "$NAME" "$WANT" || echo "   also searched for $NAME, which server/src/admin/config.rs doesn't name"
      done
    fi
  fi
  unset KEYS_OUT GOOD NAMED VALUES
fi
[ "$KEYS_TRACE" = 0 ] || set -x

echo "== large or binary files"
(cd "$OUT" && git ls-files -z | xargs -0 du -k 2>/dev/null | sort -rn | sed -n 1,5p | sed 's/^/   /')

[ "$FAIL" = 0 ] || echo "== FAILED: fix the findings above in the repository, commit, and run again" >&2
[ "$KEYS_UNCHECKED" = 0 ] || echo "== NOT VERIFIED, don't push: the built-in keys weren't fully checked; see \"the project's own keys\" above, then run again" >&2
[ "$FAIL" = 0 ] || exit 1
[ "$KEYS_UNCHECKED" = 0 ] || exit 3
[ "$KEYS_SKIPPED" = 0 ] || echo "   (the built-in key check was skipped: this clone has no builtin-keys.sh)"
[ "$USER_PATTERNS_MISSING" = 0 ] || echo "   (only the generic private-detail patterns ran: no personal patterns file at $USER_PATTERNS)"
echo "== OK: $OUT is ready to push (git -C $OUT remote add origin …)"
