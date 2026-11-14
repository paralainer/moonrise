#!/usr/bin/env bash
# The build Mac's simulator lock (sim_lock in scripts/common.sh, scripts/sim-lock.sh on the Mac),
# offline, through scripts/test-apple.sh: several test runs at once made the Mac swap until the
# timing tests failed, so one run at a time.
# - two runs started together run one after the other, and the second says it is waiting;
# - a lock whose holder is gone (or was killed with -9) doesn't hold anyone up, and four runs at
#   once on such a lock hold it one at a time;
# - the lock is released when the tests fail and when the run is stopped (Ctrl-C);
# - a run that waits longer than SIM_LOCK_WAIT gives up, without copying or testing anything.
# Needs perl, as the Mac's side does.
# ssh is a stub that runs the lock's script here, with HOME a scratch "Mac" folder; the tests'
# command just logs when it starts and ends. rsync is a stub too.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
C=$(fresh_clone)
MACHOME="$SCRATCH/mac"; LOCK="$MACHOME/.moonrise-sim-lock"
EVENTS="$SCRATCH/events"; export EVENTS
mkdir -p "$SCRATCH/bin" "$MACHOME"
cat > "$SCRATCH/bin/ssh" <<STUB
#!/bin/bash
while [ \$# -gt 0 ]; do case "\$1" in -o) shift 2 ;; -*) shift ;; *) break ;; esac; done
shift
[ "\$*" = true ] && exit 0
case "\$*" in
  *" sim-lock "*) cd "$MACHOME" && HOME="$MACHOME" exec bash -c "\$*" ;;
esac
# The tests: logged with this run's name, RUN.
echo "start \$RUN" >> "\$EVENTS"
sleep "\${TEST_SECONDS:-2}"
echo "end \$RUN" >> "\$EVENTS"
exit "\${TEST_EXIT:-0}"
STUB
printf '#!/bin/bash\necho "rsync $RUN" >> "$EVENTS"\n' > "$SCRATCH/bin/rsync"
chmod +x "$SCRATCH/bin/"*
run() {  # run <name>: scripts/test-apple.sh with ONLY (the unit tests' ssh alone)
  RUN=$1 PATH="$SCRATCH/bin:$PATH" MAC=tester@mac.invalid ONLY=SegmentRulesTests MAC_SSH_BACKOFF=0 \
    bash "$C/scripts/test-apple.sh" 2>&1
}
events() { tr '\n' ' ' < "$EVENTS" | sed 's/ $//'; }
# locked_now: whether someone holds the lock (the kernel's, on the file).
locked_now() { ! perl -MFcntl=:flock -e 'open(my $f, ">>", $ARGV[0]) or exit 0; exit(flock($f, LOCK_EX | LOCK_NB) ? 0 : 1)' "$LOCK"; }
unlocked() { if locked_now; then fail "$1: the lock is still held" "$(cat "$LOCK" 2>/dev/null)"; else ok; fi; }
# hold <who>: the lock's script, run here as the Mac would, holds the lock in the background until
# file descriptor 8 closes; HOLD_PID is its PID.
hold() {
  rm -f "$SCRATCH/hold.in"; mkfifo "$SCRATCH/hold.in"
  HOME="$MACHOME" bash "$C/scripts/sim-lock.sh" 60 "$1" < "$SCRATCH/hold.in" > "$SCRATCH/hold.out" & HOLD_PID=$!
  exec 8> "$SCRATCH/hold.in"
  for _ in $(seq 1 50); do grep -qx locked "$SCRATCH/hold.out" && return 0; sleep 0.1; done
  fail "hold: the lock wasn't taken"
}
# hold_briefly <name>: takes the lock as hold does, holds it for 0.3 s, logging "start <name>" once
# it holds it and "end <name>" before it lets go, to $SCRATCH/holds.
hold_briefly() {
  local in="$SCRATCH/in.$1" out="$SCRATCH/out.$1" p
  rm -f "$in"; mkfifo "$in"
  HOME="$MACHOME" bash "$C/scripts/sim-lock.sh" 60 "$1" < "$in" > "$out" 2>/dev/null & p=$!
  exec 7> "$in"
  until grep -qx locked "$out" 2>/dev/null; do kill -0 $p 2>/dev/null || { exec 7>&-; return 1; }; sleep 0.02; done
  echo "start $1" >> "$SCRATCH/holds"
  sleep 0.3
  echo "end $1" >> "$SCRATCH/holds"
  exec 7>&-
  wait $p
}

# One run: the lock is taken before the copy and gone afterwards.
: > "$EVENTS"
out=$(run one); rc=$?
expect_status 0 $rc "one run" "$out"
[ "$(events)" = "rsync one start one end one" ] && ok || fail "one run: events $(events)" "$out"
unlocked "after one run"

# Two at once: the second waits for the first, and says so.
: > "$EVENTS"
run a > "$SCRATCH/a.out" & a=$!
sleep 1
run b > "$SCRATCH/b.out" & b=$!
wait $a; rca=$?; wait $b; rcb=$?
expect_status 0 $rca "the first of two runs" "$(cat "$SCRATCH/a.out")"
expect_status 0 $rcb "the second of two runs" "$(cat "$SCRATCH/b.out")"
[ "$(events)" = "rsync a start a end a rsync b start b end b" ] && ok \
  || fail "two runs at once overlapped: events $(events)" "$(cat "$SCRATCH/b.out")"
expect_in "waiting for the build Mac's simulators: in use by test-apple.sh ONLY=SegmentRulesTests" "$(cat "$SCRATCH/b.out")" "the second says it waits, and for what"
unlocked "after two runs"

# A lock left by a run that is gone: its file says who held it, but nobody holds the kernel lock,
# so the next run takes it at once.
bash -c 'exit 0' & dead=$!; wait $dead
printf '%s\n%s\n%s\n' "$dead" "$(date +%s)" "an old run" > "$LOCK"
: > "$EVENTS"
out=$(run stale); rc=$?
expect_status 0 $rc "a lock whose holder is gone" "$out"
expect_not_in "waiting" "$out" "a lock whose holder is gone: no wait"
[ "$(events)" = "rsync stale start stale end stale" ] && ok || fail "stale lock: events $(events)" "$out"
unlocked "after a lock whose holder is gone"

# A held lock: a run gives up after SIM_LOCK_WAIT, having copied and tested nothing, and the holder
# keeps it.
hold "a long run"
: > "$EVENTS"
out=$(SIM_LOCK_WAIT=4 run late); rc=$?
expect_status 1 $rc "a run that waits too long" "$out"
expect_in "still in use (a long run) after 4 s of waiting: giving up" "$out" "says it gave up"
[ ! -s "$EVENTS" ] && ok || fail "a run that gave up still ran: events $(events)" "$out"
locked_now && ok || fail "the holder lost the lock to a run that gave up"
# A holder killed outright (kill -9: no trap, no clean-up) leaves nothing that blocks the next run.
kill -9 "$HOLD_PID"; wait "$HOLD_PID" 2>/dev/null; exec 8>&-
out=$(run after-kill); rc=$?
expect_status 0 $rc "the run after a holder killed with -9" "$out"
expect_not_in "waiting" "$out" "the run after a holder killed with -9 doesn't wait"

# Several runs at once on a lock left by a run that is gone (the case that once let two of them
# hold it): in every trial, no two hold periods overlap.
overlaps=0
for trial in $(seq 1 "${TRIALS:-8}"); do
  printf '%s\n%s\n%s\n' 999999 "$(date +%s)" "a dead run" > "$LOCK"
  : > "$SCRATCH/holds"
  for w in $(seq 1 "${WAITERS:-4}"); do hold_briefly "w$w" & done
  wait
  overlaps=$((overlaps + $(awk '$1 == "start" { n++; if (n > 1) bad++ } $1 == "end" { n-- } END { print bad + 0 }' "$SCRATCH/holds")))
  [ "$(grep -c "^end" "$SCRATCH/holds")" = "${WAITERS:-4}" ] || { overlaps=$((overlaps + 1)); echo "trial $trial: not every waiter held it" >&2; }
done
[ "$overlaps" = 0 ] && ok || fail "${WAITERS:-4} runs at once on a lock left behind: $overlaps overlapping holds in ${TRIALS:-8} trials"
unlocked "after the runs on a lock left behind"

# Released when the tests fail, so the next run doesn't wait.
: > "$EVENTS"
out=$(TEST_EXIT=1 run failing); rc=$?
expect_status 1 $rc "failing tests" "$out"
unlocked "after failing tests"
out=$(run next); rc=$?
expect_status 0 $rc "the run after failing tests" "$out"
expect_not_in "waiting" "$out" "the run after failing tests doesn't wait"

# Released when the run is stopped with Ctrl-C (SIGINT to its process group) mid-tests.
: > "$EVENTS"
set -m
TEST_SECONDS=30 run stopped > "$SCRATCH/stopped.out" & s=$!
set +m
for _ in $(seq 1 50); do grep -q "start stopped" "$EVENTS" && break; sleep 0.2; done
locked_now && ok || fail "no lock while the tests run" "$(cat "$SCRATCH/stopped.out")"
kill -INT -- -$s 2>/dev/null
wait $s 2>/dev/null
for _ in $(seq 1 50); do locked_now || break; sleep 0.2; done
unlocked "after Ctrl-C"

done_testing
