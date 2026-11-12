#!/usr/bin/env bash
# The tracked pre-push hook (.githooks/pre-push) and `make hooks`. In a scratch repository
# with a scratch bare remote: a clean branch goes through, a push to main is refused, and so is a
# branch carrying a private detail (a made-up pattern from a scratch private-patterns.txt) in a
# file, a commit message or an identity. The user's patterns file: missing at its usual place, the
# hook says so and checks only the generic patterns; named by MOONRISE_PRIVATE_PATTERNS but missing
# or unreadable, the push is refused; present (either way), its patterns are caught. HOME is a
# scratch folder, so the real per-user file never takes part. No real private value is used.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

R="$SCRATCH/repo"; REMOTE="$SCRATCH/remote.git"
git init -q --bare "$REMOTE"
git init -q -b main "$R"
mkdir -p "$R/scripts" "$R/.githooks"
cp "$REPO/.githooks/pre-push" "$R/.githooks/"
cp "$REPO/scripts/export-public.sh" "$R/scripts/"
cp "$REPO/Makefile" "$R/"
G() { git -C "$R" -c user.name=Tester -c user.email=tester@example.com -c commit.gpgsign=false "$@"; }
G remote add origin "$REMOTE"
# The user's own patterns file: a scratch HOME without one, so the real one never takes part.
export HOME="$SCRATCH/home"; mkdir -p "$HOME"
unset XDG_CONFIG_HOME MOONRISE_PRIVATE_PATTERNS
DEFAULT_PATTERNS="$HOME/.config/moonrise/private-patterns.txt"

# --- make hooks sets core.hooksPath in this clone.
if command -v make >/dev/null; then
  out=$(make -s -C "$R" hooks 2>&1); rc=$?
  expect_status 0 $rc "make hooks" "$out"
  [ "$(git -C "$R" config --get core.hooksPath)" = .githooks ] && ok || fail "make hooks didn't set core.hooksPath"
  out=$(make -n -C "$R" check-quick 2>&1)
  expect_in "make hooks" "$out" "make check-quick carries the hooks reminder"
else
  echo "git-hooks.test.sh: no make here, make hooks not checked"
  git -C "$R" config core.hooksPath .githooks
fi

echo hello > "$R/README"
G add -A; G commit -q -m "first"
# A first push to main is refused; the override lets the base in.
out=$(G push -q origin main 2>&1); rc=$?
[ $rc -ne 0 ] && ok || fail "a push to main went through"
expect_in "main takes changes through pull requests" "$out" "the main refusal says why"
out=$(MOONRISE_ALLOW_MAIN_PUSH=1 G push -q origin main 2>&1); rc=$?
expect_status 0 $rc "MOONRISE_ALLOW_MAIN_PUSH=1 lets main through" "$out"

# --- A clean branch goes through, without any private-patterns.txt.
G checkout -q -b clean
echo "more" >> "$R/README"; G commit -qam "clean change"
out=$(G push -q origin clean 2>&1); rc=$?
expect_status 0 $rc "a clean branch, no private-patterns.txt" "$out"
expect_in "no personal patterns file at $DEFAULT_PATTERNS" "$out" "a missing user file is said"
expect_in "only the generic patterns" "$out" "the notice says what is checked"

# --- A private detail from scripts/local/private-patterns.txt, in a file, a message, an identity.
mkdir -p "$R/scripts/local"
printf '# made up for this test\nzq-fake-private-[0-9]{4}\n' > "$R/scripts/local/private-patterns.txt"
FAKE=zq-fake-private-1234
G checkout -q -b leak-file clean
echo "host $FAKE here" > "$R/notes.txt"; G add notes.txt; G commit -qm "add notes"
out=$(G push -q origin leak-file 2>&1); rc=$?
[ $rc -ne 0 ] && ok || fail "a private detail in a file went through"
expect_in "private details in the tree" "$out" "file hit reported"
expect_in "notes.txt:1:6" "$out" "file hit gives the place"
expect_in "private-patterns.txt line 2" "$out" "file hit gives the pattern's label"
expect_not_in "$FAKE" "$out" "the match itself isn't printed"
[ -z "$(git -C "$REMOTE" for-each-ref refs/heads/leak-file)" ] && ok || fail "the refused branch reached the remote"

G checkout -q -b leak-msg clean
echo "x" >> "$R/README"; G commit -qam "talk about $FAKE"
out=$(G push -q origin leak-msg 2>&1); rc=$?
[ $rc -ne 0 ] && ok || fail "a private detail in a commit message went through"
expect_in "commit messages or identities" "$out" "message hit reported"
expect_in "message line 1" "$out" "message hit gives the line"

G checkout -q -b leak-ident clean
echo "y" >> "$R/README"
git -C "$R" -c user.name="$FAKE" -c user.email=tester@example.com -c commit.gpgsign=false commit -qam "plain"
out=$(G push -q origin leak-ident 2>&1); rc=$?
[ $rc -ne 0 ] && ok || fail "a private detail in an identity went through"
expect_in "identities" "$out" "identity hit reported"

# --- The user's patterns file (outside the checkout).
# Missing at its usual place: only the generic patterns run, so a value only it would catch goes
# through, with the notice.
G checkout -q -b user-generic clean
echo "zq-user-only-""111" > "$R/user.txt"; G add user.txt; G commit -qm "user generic"
out=$(G push -q origin user-generic 2>&1); rc=$?
expect_status 0 $rc "no user file: only the generic patterns" "$out"
expect_in "no personal patterns file at $DEFAULT_PATTERNS" "$out" "the notice, on that push"
# Named by MOONRISE_PRIVATE_PATTERNS but not there: the push is refused, whatever it carries.
G checkout -q -b named-missing clean
echo "z" >> "$R/README"; G commit -qam "clean again"
out=$(MOONRISE_PRIVATE_PATTERNS="$SCRATCH/no-such-patterns.txt" G push -q origin named-missing 2>&1); rc=$?
[ $rc -ne 0 ] && ok || fail "a push went through with MOONRISE_PRIVATE_PATTERNS naming a missing file"
expect_in "MOONRISE_PRIVATE_PATTERNS names $SCRATCH/no-such-patterns.txt" "$out" "the refusal names the file"
expect_in "Push refused" "$out" "the refusal says so"
[ -z "$(git -C "$REMOTE" for-each-ref refs/heads/named-missing)" ] && ok || fail "the refused branch reached the remote"
# Named, there, but unreadable: refused too (not checkable as root, who reads anything).
if [ "$(id -u)" != 0 ]; then
  printf 'zq-user-only-[0-9]{3}\n' > "$SCRATCH/unreadable.txt"; chmod 000 "$SCRATCH/unreadable.txt"
  out=$(MOONRISE_PRIVATE_PATTERNS="$SCRATCH/unreadable.txt" G push -q origin named-missing 2>&1); rc=$?
  [ $rc -ne 0 ] && ok || fail "a push went through with MOONRISE_PRIVATE_PATTERNS naming an unreadable file"
  expect_in "can't be read" "$out" "the unreadable file's refusal"
  rm -f "$SCRATCH/unreadable.txt"
fi
# Present at its usual place: its pattern is caught, and there is no notice.
mkdir -p "$(dirname "$DEFAULT_PATTERNS")"
printf 'zq-user-only-[0-9]{3}    # made up for this test\n' > "$DEFAULT_PATTERNS"
G checkout -q -b leak-default clean
echo "zq-user-only-""222" > "$R/user.txt"; G add user.txt; G commit -qm "user default"
out=$(G push -q origin leak-default 2>&1); rc=$?
[ $rc -ne 0 ] && ok || fail "a match of the user file at its usual place went through"
expect_in "your private-patterns.txt line 1" "$out" "user file hit labelled"
expect_not_in "no personal patterns file" "$out" "no notice when the file is there"
rm -f "$DEFAULT_PATTERNS"
# Named by MOONRISE_PRIVATE_PATTERNS and there: its pattern is caught.
printf '# made up for this test\nzq-user-only-[0-9]{3}\n' > "$SCRATCH/user-patterns.txt"
G checkout -q -b leak-user clean
echo "zq-user-only-""123" > "$R/user.txt"; G add user.txt; G commit -qm "user"
out=$(MOONRISE_PRIVATE_PATTERNS="$SCRATCH/user-patterns.txt" G push -q origin leak-user 2>&1); rc=$?
[ $rc -ne 0 ] && ok || fail "a match of the named user file went through"
expect_in "your private-patterns.txt line 2" "$out" "named user file hit labelled"

# --- The tracked patterns in export-public.sh apply too; the hook names the pattern by its label.
G checkout -q -b leak-tracked clean
# Spelled in two pieces so this file doesn't match the pattern itself.
echo "see op:/""/vault/item" > "$R/doc.txt"; G add doc.txt; G commit -qm "doc"
out=$(G push -q origin leak-tracked 2>&1); rc=$?
[ $rc -ne 0 ] && ok || fail "a tracked pattern's match went through"
expect_in "the password manager" "$out" "tracked pattern labelled from export-public.sh"

done_testing
