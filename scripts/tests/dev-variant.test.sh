#!/usr/bin/env bash
# The scripts' DEV=1 (the Dev app, "Moonrise Dev"):
# - scripts/ota.sh and scripts/release-apple.sh ship the app itself only: with DEV=1 they stop at once
#   (exit 2, saying so), before anything reaches the build Mac; without it they go on as before.
# - scripts/sim.sh and scripts/tvsim.sh (sim_launch in common.sh) wait for the launch until
#   simctl says the app is up, launch once more when it fails or doesn't come up, and then fail
#   (exit 1) with simctl's output. Here `ssh` runs the Mac's side of it on this machine, with a stub
#   `xcrun` whose app comes up at the STUB_RUNS_AT'th launch (never, when 0): before that a launch
#   fails as simctl does, or with STUB_HANG=1 hangs, saying nothing. Its launchctl list always
#   has the other app (the release one), so only the right bundle id counts.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
C=$(fresh_clone)
mkdir -p "$SCRATCH/bin"
LOG="$SCRATCH/calls.log"; export LOG SCRATCH
for t in rsync scp; do printf '#!/bin/bash\necho "%s $*" >> "$LOG"\n' "$t" > "$SCRATCH/bin/$t"; done
printf '#!/bin/bash\nexit 0\n' > "$SCRATCH/bin/sleep"
cat > "$SCRATCH/bin/ssh" <<'STUB'
#!/bin/bash
# ssh <mac> <command>: logged, then run here by bash, with /tmp/ moved into the scratch folder.
echo "ssh $1" >> "$LOG"
shift
cmd=$*
exec bash -c "${cmd//\/tmp\//$SCRATCH/}"
STUB
cat > "$SCRATCH/bin/xcrun" <<'STUB'
#!/bin/bash
echo "xcrun $*" >> "$LOG"
n=$(grep -c '^xcrun simctl launch' "$LOG")
case "$*" in
  "simctl launch "*)
    if [ "${STUB_RUNS_AT:-1}" != 0 ] && [ "$n" -ge "${STUB_RUNS_AT:-1}" ]; then
      echo "${*: -1}: 456"; touch "$SCRATCH/app-up"; exit 0
    fi
    [ "${STUB_HANG:-}" = 1 ] && exec /bin/sleep 30
    echo "An error was encountered processing the command (domain=FBSOpenApplicationServiceErrorDomain, code=4):"
    echo "Simulator device failed to launch ${*: -1}."; exit 4 ;;
  "simctl spawn "*" launchctl list")
    echo "123 0 UIKitApplication:org.example.moonrise[a1b2][rb-legacy]"
    [ -e "$SCRATCH/app-up" ] && echo "456 0 UIKitApplication:org.example.moonrise.dev[c3d4][rb-legacy]" ;;
esac
exit 0
STUB
chmod +x "$SCRATCH/bin/"*
unset DEV MAC FASTLANE_MAC RELEASE_MAC PERSONAL_MACS
run() { rm -f "$LOG" "$SCRATCH/app-up"; touch "$LOG"; PATH="$SCRATCH/bin:$PATH" bash "$@" 2>&1; }

for s in ota.sh "release-apple.sh ios"; do
  out=$(DEV=1 MAC=nobody@example.invalid RELEASE_MAC=nobody@example.invalid run "$C/scripts/"$s); rc=$?
  expect_status 2 $rc "DEV=1 scripts/$s" "$out"
  expect_in "scripts/${s%% *} ships Moonrise itself, not the Dev app" "$out" "DEV=1 $s says why"
  [ -s "$LOG" ] && fail "DEV=1 $s reached the build Mac" "$(cat "$LOG")" || ok
  out=$(DEV=1 run "$C/scripts/"$s); rc=$?
  expect_status 2 $rc "DEV=1 scripts/$s with no build Mac set" "$out"
  # Without DEV=1 (and with the App Store id), the same fresh clone goes on to the usual checks
  # (here: no signing set up).
  out=$(MAC=nobody@example.invalid RELEASE_MAC=nobody@example.invalid STORE_BUNDLE_ID=org.example.store run "$C/scripts/"$s); rc=$?
  expect_status 1 $rc "scripts/$s without DEV=1, in a fresh clone" "$out"
  expect_not_in "not the Dev app" "$out" "$s without DEV=1 isn't refused"
  expect_in "signs the app" "$out" "$s without DEV=1 reaches the signing check"
done

launches() { grep -c '^xcrun simctl launch' "$LOG"; }
for script in sim.sh tvsim.sh; do
  sim() { DEV=1 SEED=0 SIM=TESTSIM MAC=nobody@example.invalid LAUNCH_WAIT=${W:-3} STUB_RUNS_AT=$1 STUB_HANG=${2:-} run "$C/scripts/$script"; }

  out=$(sim 1); rc=$?
  expect_status 0 $rc "$script, the app starts at once" "$out"
  expect_in "launched" "$out" "$script says launched"
  [ "$(launches)" = 1 ] && ok || fail "$script: an app up after one launch was launched $(launches) times" "$(cat "$LOG")"
  expect_in "xcrun simctl bootstatus TESTSIM -b" "$(cat "$LOG")" "$script boots the simulator fully first"
  expect_in "launch --console-pty TESTSIM org.example.moonrise.dev" "$(cat "$LOG")" "$script: DEV=1 launches the Dev app"

  out=$(sim 2); rc=$?
  expect_status 0 $rc "$script, the first launch fails, the second starts the app" "$out"
  expect_in "launching again" "$out" "$script says it launches again"
  expect_in "failed to launch org.example.moonrise.dev" "$out" "$script shows why the first launch failed"
  expect_in "launched" "$out" "then that it launched"
  [ "$(launches)" = 2 ] && ok || fail "$script: the app was launched $(launches) times, wanted 2" "$(cat "$LOG")"

  out=$(W=8 sim 2 1); rc=$?
  expect_status 0 $rc "$script, the first launch hangs (stopped after LAUNCH_WAIT), the second starts the app" "$out"
  expect_in "waiting for the app to start" "$out" "$script says it waits"
  [ "$(launches)" = 2 ] && ok || fail "$script: the app was launched $(launches) times, wanted 2" "$(cat "$LOG")"

  out=$(sim 0); rc=$?
  expect_status 1 $rc "$script, the app never starts (an error now, not a warning)" "$out"
  expect_in "error: org.example.moonrise.dev did not start" "$out" "$script names the Dev app (the release app running doesn't count)"
  expect_in "Simulator device failed to launch" "$out" "$script shows simctl's error"
  [ "$(launches)" = 2 ] && ok || fail "$script: the app was launched $(launches) times, wanted 2 (one retry only)" "$(cat "$LOG")"
done
done_testing
