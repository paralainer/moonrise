#!/usr/bin/env bash
# scripts/export-public.sh on a small scratch repository (a copy of the script, generic files):
#  - a private-details hit gives file:line:column, never the line (the rest could be a key);
#  - more than 80 hits still runs every scan to the end and exits 1 (not 141 from SIGPIPE);
#  - with a builtin-keys.sh helper that fails, leaves a key out or prints odd lines, the run
#    is "NOT VERIFIED" (exit 3), never OK; under bash -x no key value is printed;
#  - the repository's .gitleaks.toml lets file_key hashes through in server/testdata/golden/
#    (golden test output), while a real-looking key elsewhere, or in a golden file on a line
#    that isn't a file_key, still fails the run;
#  - the user's patterns file: missing at its usual place, the run says so (also on the closing
#    lines) and checks only the generic patterns; named by MOONRISE_PRIVATE_PATTERNS but missing
#    or unreadable, the run stops with exit 4; present (either way), its patterns are caught.
# The private patterns are made-up ones, in the scratch repo's scripts/local/private-patterns.txt
# and in scratch user files. HOME is a scratch folder, so the real per-user file never takes part.
# gitleaks runs in Docker from the local image; without it (or Docker) this test is skipped.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
GITLEAKS=$(sed -n 's/.*\(zricethezav\/gitleaks:[^ ]*\).*/\1/p' "$REPO/scripts/export-public.sh" | head -1)
if ! docker image inspect "$GITLEAKS" >/dev/null 2>&1; then
  echo "export-public.test.sh: skipped (no local $GITLEAKS image; docker pull it once)"
  exit 0
fi

# The user's own patterns file: a scratch HOME, so the real one never takes part, and a scratch
# file named by MOONRISE_PRIVATE_PATTERNS for most cases.
export HOME="$SCRATCH/home"; mkdir -p "$HOME"
unset XDG_CONFIG_HOME
DEFAULT_PATTERNS="$HOME/.config/moonrise/private-patterns.txt"
export MOONRISE_PRIVATE_PATTERNS="$SCRATCH/user-patterns.txt"
printf 'zquseronly[0-9]+    # made up for this test\n' > "$MOONRISE_PRIVATE_PATTERNS"

# make_repo <dir>: the scratch source repository, committed.
make_repo() {
  mkdir -p "$1/scripts/local" "$1/server/src/admin" "$1/docs"
  cp "$REPO/scripts/export-public.sh" "$1/scripts/"
  cp "$REPO/.gitleaks.toml" "$1/"
  printf 'const KEY: &str = "MOONRISE_BUILTIN_ALPHA_KEY";\nconst OTHER: &str = "MOONRISE_BUILTIN_BETA_KEY";\n' > "$1/server/src/admin/config.rs"
  printf '# Notes\nNothing private here.\n' > "$1/docs/notes.md"
  printf 'zqxprivate[0-9]+    # a made-up private detail\n' > "$1/scripts/local/private-patterns.txt"
  printf 'scripts/local/\nserver/deploy/local/\n' > "$1/.gitignore"
}
commit() { git -C "$1" add -A && git -C "$1" -c user.name=t -c user.email=t@example.com -c commit.gpgsign=false commit -qm "$2"; }
export_to() { bash "$1/scripts/export-public.sh" "$SCRATCH/out-$RANDOM" 2>&1; }

# A clean repository passes, the key check skipped (no helper) and said so.
R="$SCRATCH/clean"; make_repo "$R"; git -C "$R" init -q; commit "$R" init
out=$(export_to "$R"); rc=$?
expect_status 0 $rc "a clean repository" "$out"
expect_in "the built-in key check was skipped" "$out" "a skipped key check is said"

# The line of a hit is never printed.
R="$SCRATCH/hit"; make_repo "$R"; git -C "$R" init -q
printf 'intro\nsee zqxprivate42 next to TAILTEXTNOTSHOWN\n' > "$R/docs/a.txt"; commit "$R" hit
out=$(export_to "$R"); rc=$?
expect_status 1 $rc "a private detail fails the run" "$out"
expect_in "docs/a.txt:2:5" "$out" "the hit's place"
expect_not_in "TAILTEXTNOTSHOWN" "$out" "the rest of the line"
expect_not_in "zqxprivate42" "$out" "the match itself"

# A pattern from the user's file (outside the checkout) is scanned for too.
R="$SCRATCH/userhit"; make_repo "$R"; git -C "$R" init -q
printf 'see zquseronly7 here\n' > "$R/docs/u.txt"; commit "$R" userhit
out=$(export_to "$R"); rc=$?
expect_status 1 $rc "a match of the user's patterns file fails the run" "$out"
expect_in "your private-patterns.txt line 1" "$out" "the user file's label"
expect_not_in "no personal patterns file" "$out" "no notice when the named file is there"

# The user file missing at its usual place: said, in the scan and on the closing lines, and only
# the generic patterns run, so a value only it would catch passes.
out=$(env -u MOONRISE_PRIVATE_PATTERNS bash "$R/scripts/export-public.sh" "$SCRATCH/out-$RANDOM" 2>&1); rc=$?
expect_status 0 $rc "no user file: only the generic patterns" "$out"
expect_in "no personal patterns file at $DEFAULT_PATTERNS" "$out" "a missing user file is said"
expect_in "(only the generic private-detail patterns ran: no personal patterns file at $DEFAULT_PATTERNS)" "$out" "and on the closing lines"
# The user file present at its usual place: its pattern is caught, with no notice.
mkdir -p "$(dirname "$DEFAULT_PATTERNS")"; cp "$MOONRISE_PRIVATE_PATTERNS" "$DEFAULT_PATTERNS"
out=$(env -u MOONRISE_PRIVATE_PATTERNS bash "$R/scripts/export-public.sh" "$SCRATCH/out-$RANDOM" 2>&1); rc=$?
expect_status 1 $rc "a match of the user file at its usual place" "$out"
expect_in "your private-patterns.txt line 1" "$out" "the usual place's label"
expect_not_in "no personal patterns file" "$out" "no notice when the file is there"
rm -f "$DEFAULT_PATTERNS"
# Named by MOONRISE_PRIVATE_PATTERNS but not there, or unreadable: exit 4, nothing exported.
O="$SCRATCH/out-named-missing"
out=$(MOONRISE_PRIVATE_PATTERNS="$SCRATCH/no-such-patterns.txt" bash "$R/scripts/export-public.sh" "$O" 2>&1); rc=$?
expect_status 4 $rc "MOONRISE_PRIVATE_PATTERNS naming a missing file" "$out"
expect_in "MOONRISE_PRIVATE_PATTERNS names $SCRATCH/no-such-patterns.txt" "$out" "the error names the file"
[ ! -e "$O" ] && ok || fail "the run exported although its patterns file was missing"
if [ "$(id -u)" != 0 ]; then
  cp "$MOONRISE_PRIVATE_PATTERNS" "$SCRATCH/unreadable.txt"; chmod 000 "$SCRATCH/unreadable.txt"
  out=$(MOONRISE_PRIVATE_PATTERNS="$SCRATCH/unreadable.txt" bash "$R/scripts/export-public.sh" "$SCRATCH/out-$RANDOM" 2>&1); rc=$?
  expect_status 4 $rc "MOONRISE_PRIVATE_PATTERNS naming an unreadable file" "$out"
  expect_in "can't be read" "$out" "the unreadable file's error"
  rm -f "$SCRATCH/unreadable.txt"
fi

# 120 hits: cut to 80 in the report, every scan still runs, exit 1.
R="$SCRATCH/many"; make_repo "$R"; git -C "$R" init -q
for i in $(seq 1 120); do echo "zqxprivate$i"; done > "$R/docs/many.txt"; commit "$R" many
out=$(export_to "$R"); rc=$?
expect_status 1 $rc "120 hits" "$out"
expect_in "(120 matches;" "$out" "all hits counted"
expect_in "== large or binary files" "$out" "the last scan ran"

# Golden output with a file_key passes; a real-looking key fails, in another file or on
# another line of a golden file. The values are made at run time, so this file has none to flag.
# A file_key like the golden files' (all 16 hex digits, so high entropy): gitleaks flags it without
# the allowlist, which the run with no .gitleaks.toml below confirms.
hex16=$(printf '%x' {0..15} | fold -w1 | shuf | tr -d '\n')
fake_key=$(printf 'golden-fake' | base64 | tr -d '=')Xq9Zr7Lm2Pw4   # mixed case: gitleaks skips plain hex
golden() { mkdir -p "$1/server/testdata/golden/media_load"; printf 'Probe {\n    file_key: "%s",\n}\n' "$hex16" > "$1/server/testdata/golden/media_load/probe.txt"; }
R="$SCRATCH/golden"; make_repo "$R"; git -C "$R" init -q; golden "$R"; commit "$R" golden
out=$(export_to "$R"); rc=$?
expect_status 0 $rc "a file_key in golden test output" "$out"
expect_in "no leaks" "$out" "gitleaks passes the golden file_key"
git -C "$R" rm -q .gitleaks.toml; commit "$R" "no allowlist"
out=$(export_to "$R"); rc=$?
expect_status 1 $rc "the golden file_key without .gitleaks.toml" "$out"
R="$SCRATCH/golden-leak"; make_repo "$R"; git -C "$R" init -q; golden "$R"
printf 'api_key = "%s"\n' "$fake_key" > "$R/docs/config.txt"; commit "$R" leak
out=$(export_to "$R"); rc=$?
expect_status 1 $rc "a real-looking key outside the golden files" "$out"
expect_in "leaks found: 1" "$out" "gitleaks flags the key, not the file_key"
expect_not_in "$fake_key" "$out" "the key's value"
R="$SCRATCH/golden-other"; make_repo "$R"; git -C "$R" init -q; golden "$R"
printf '    api_key: "%s",\n' "$fake_key" >> "$R/server/testdata/golden/media_load/probe.txt"; commit "$R" leak
out=$(export_to "$R"); rc=$?
expect_status 1 $rc "a real-looking key on another line of a golden file" "$out"
expect_in "leaks found: 1" "$out" "gitleaks flags the golden file's other key"

# A key helper that fails, leaves a key out, or prints other lines: NOT VERIFIED, exit 3.
helper() {
  R="$SCRATCH/keys-$RANDOM"; make_repo "$R"; git -C "$R" init -q; commit "$R" init
  mkdir -p "$R/server/deploy/local"
  printf '#!/bin/sh\n%s\n' "$1" > "$R/server/deploy/local/builtin-keys.sh"; chmod +x "$R/server/deploy/local/builtin-keys.sh"
  printf '%s\n' "$R"
}
R=$(helper 'exit 4'); out=$(export_to "$R"); rc=$?
expect_status 3 $rc "a failing key helper" "$out"
expect_in "NOT VERIFIED" "$out" "a failing helper is not OK"
R=$(helper 'echo MOONRISE_BUILTIN_ALPHA_KEY=fakevalue-aaaa1111'); out=$(export_to "$R"); rc=$?
expect_status 3 $rc "a helper that leaves a key out" "$out"
expect_in "no value for MOONRISE_BUILTIN_BETA_KEY" "$out" "names the missing key"
R=$(helper 'printf "MOONRISE_BUILTIN_ALPHA_KEY=fakevalue-aaaa1111\nMOONRISE_BUILTIN_BETA_KEY=\"quoted value\"\nstray-line\n"'); out=$(export_to "$R"); rc=$?
expect_status 3 $rc "odd helper output" "$out"
expect_not_in "fakevalue-aaaa1111" "$out" "no key value"
# A good helper: checked and OK; under bash -x still no value in the output.
R=$(helper 'printf "MOONRISE_BUILTIN_ALPHA_KEY=fakevalue-aaaa1111\nMOONRISE_BUILTIN_BETA_KEY=fakevalue-bbbb2222\n"')
out=$(export_to "$R"); rc=$?
expect_status 0 $rc "a good key helper" "$out"
expect_in "keys checked: 2" "$out" "both keys searched for"
out=$(bash -x "$R/scripts/export-public.sh" "$SCRATCH/out-x" 2>&1); rc=$?
expect_status 0 $rc "under bash -x" ""
expect_not_in "fakevalue-" "$out" "no key value under bash -x"
# A key that is in the copy is found.
echo "token fakevalue-bbbb2222" > "$R/docs/leak.txt"; commit "$R" leak
out=$(export_to "$R"); rc=$?
expect_status 1 $rc "a built-in key in the copy" "$out"
expect_in "a built-in key is in: docs/leak.txt" "$out" "names the file"
expect_not_in "fakevalue-" "$out" "never the value"
done_testing
