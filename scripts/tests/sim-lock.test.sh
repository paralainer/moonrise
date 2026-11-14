#!/usr/bin/env bash
# The build Mac's simulator lock (sim_lock in scripts/common.sh, scripts/sim-lock.sh on the Mac),
# offline, through scripts/test-apple.sh: several test runs at once made the Mac swap until the
# timing tests failed, so one run at a time.
# - two runs started together run one after the other, and the second says it is waiting;
# - a lock whose holder is gone, or that is older than SIM_LOCK_MAX_AGE, is taken over;
# - the lock is released when the tests fail and when the run is stopped (Ctrl-C);
# - a run that waits longer than SIM_LOCK_WAIT gives up, without copying or testing anything.
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
unlocked() { if [ -e "$LOCK" ]; then fail "$1: the lock is still there" "$(cat "$LOCK/owner" 2>/dev/null)"; else ok; fi; }

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

# A stale lock: its holder is gone.
mkdir -p "$LOCK"
bash -c 'exit 0' & dead=$!; wait $dead
printf '%s\n%s\n%s\n' "$dead" "$(date +%s)" "an old run" > "$LOCK/owner"
: > "$EVENTS"
out=$(run stale); rc=$?
expect_status 0 $rc "a lock whose holder is gone" "$out"
expect_in "holder (an old run, PID $dead) is gone: taking it over" "$out" "says it takes it over"
[ "$(events)" = "rsync stale start stale end stale" ] && ok || fail "stale lock: events $(events)" "$out"
unlocked "after taking over a stale lock"

# A stale lock: too old, though its PID runs (a PID used again after a restart).
sleep 600 & live=$!
mkdir -p "$LOCK"; printf '%s\n%s\n%s\n' "$live" 1000 "a forgotten run" > "$LOCK/owner"
out=$(run old); rc=$?
expect_status 0 $rc "a lock older than SIM_LOCK_MAX_AGE" "$out"
expect_in "is older than 14400 s: taking it over" "$out" "says it takes over an old lock"
unlocked "after taking over an old lock"

# A held lock: a run gives up after SIM_LOCK_WAIT, having copied and tested nothing.
mkdir -p "$LOCK"; printf '%s\n%s\n%s\n' "$live" "$(date +%s)" "a long run" > "$LOCK/owner"
: > "$EVENTS"
out=$(SIM_LOCK_WAIT=4 run late); rc=$?
expect_status 1 $rc "a run that waits too long" "$out"
expect_in "still in use (a long run) after 4 s of waiting: giving up" "$out" "says it gave up"
[ ! -s "$EVENTS" ] && ok || fail "a run that gave up still ran: events $(events)" "$out"
[ "$(sed -n 1p "$LOCK/owner" 2>/dev/null)" = "$live" ] && ok || fail "the holder's lock was removed by a run that gave up"
rm -rf "$LOCK"
kill $live 2>/dev/null; wait $live 2>/dev/null

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
[ -d "$LOCK" ] && ok || fail "no lock while the tests run" "$(cat "$SCRATCH/stopped.out")"
kill -INT -- -$s 2>/dev/null
wait $s 2>/dev/null
for _ in $(seq 1 50); do [ -e "$LOCK" ] || break; sleep 0.2; done
unlocked "after Ctrl-C"

done_testing
