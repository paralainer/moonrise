#!/usr/bin/env bash
# The build Mac's simulator lock, the Mac's side: the runs that boot simulators or build there for
# tests (test-apple.sh, check.sh's Apple builds, store-shots.sh) take it, so only one of them uses
# the Mac at a time; several at once made the Mac swap until timing tests failed. Scripts don't run
# this directly: sim_lock in scripts/common.sh sends it over ssh (before anything is copied there)
# and keeps that ssh open while it holds the lock.
#
#   sim-lock.sh <wait seconds> <max age seconds> <who>
#
# It takes the lock (a folder, ~/.moonrise-sim-lock, made with mkdir, so taking it is atomic; it
# holds the holder's PID, when it was taken and <who>), prints "locked", and holds it until its
# stdin closes: when the run is done, is stopped (Ctrl-C) or its ssh connection drops. Then it
# removes the lock, if it is still its own.
#
# A lock that is held waits, saying who holds it (on stderr), and gives up after <wait seconds>
# (exit 1). A lock is stale, and taken over, when its holder's PID is no longer running, when it is
# older than <max age seconds>, or when it has had no owner file for 30 s (a holder that died
# while taking it). Bash 3.2 (macOS's own) is enough.
set -u
WAIT=${1:?usage: sim-lock.sh <wait seconds> <max age seconds> <who>}
MAX_AGE=${2:?usage: sim-lock.sh <wait seconds> <max age seconds> <who>}
WHO=${3:-a run}
LOCK=$HOME/.moonrise-sim-lock
ME=$$
held=

release() {
  [ -n "$held" ] || return 0
  held=
  [ "$(sed -n 1p "$LOCK/owner" 2>/dev/null)" = "$ME" ] && rm -rf "$LOCK"
  return 0
}
trap release EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

take() {
  mkdir "$LOCK" 2>/dev/null || return 1
  printf '%s\n%s\n%s\n' "$ME" "$(date +%s)" "$WHO" > "$LOCK/owner.$ME" && mv "$LOCK/owner.$ME" "$LOCK/owner"
  held=1
}

# stale: whether the lock there now is stale; says why on stderr, and sets stale_pid to its
# holder's PID.
no_owner_since=
stale_pid=
stale() {
  local pid since who now
  stale_pid=
  now=$(date +%s)
  if [ ! -f "$LOCK/owner" ]; then
    [ -d "$LOCK" ] || return 1
    [ -n "$no_owner_since" ] || no_owner_since=$now
    if [ $((now - no_owner_since)) -ge 30 ]; then
      echo "the simulator lock has had no owner for 30 s: taking it over" >&2
      return 0
    fi
    return 1
  fi
  no_owner_since=
  pid=$(sed -n 1p "$LOCK/owner"); since=$(sed -n 2p "$LOCK/owner"); who=$(sed -n 3p "$LOCK/owner")
  stale_pid=$pid
  if [ -z "$pid" ] || ! kill -0 "$pid" 2>/dev/null; then
    echo "the simulator lock's holder ($who, PID $pid) is gone: taking it over" >&2
    return 0
  fi
  if [ $((now - ${since:-0})) -ge "$MAX_AGE" ]; then
    echo "the simulator lock ($who, PID $pid) is older than $MAX_AGE s: taking it over" >&2
    return 0
  fi
  return 1
}

# closed: whether our stdin has closed (the run went away while waiting). It waits up to 3 s for a
# line, so it is also the pause between tries. Bash 3.2's read returns 1 both on a timeout and at
# the end of input; the end of input returns at once, a timeout after 3 s.
closed() {
  local t=$SECONDS line
  read -r -t 3 line && return 1
  [ $((SECONDS - t)) -lt 2 ]
}

start=$SECONDS
said=
while ! take; do
  if stale; then
    # Rename it away (atomic), then check it was the stale one seen and not a lock another waiter
    # took over in between; that one goes back.
    gone=$LOCK.stale.$ME
    if mv "$LOCK" "$gone" 2>/dev/null; then
      if [ "$(sed -n 1p "$gone/owner" 2>/dev/null)" = "$stale_pid" ]; then
        rm -rf "$gone"
      else
        [ -d "$LOCK" ] || mv "$gone" "$LOCK" 2>/dev/null
      fi
    fi
    continue
  fi
  waited=$((SECONDS - start))
  if [ "$waited" -ge "$WAIT" ]; then
    echo "error: the build Mac's simulators were still in use ($(sed -n 3p "$LOCK/owner" 2>/dev/null)) after $WAIT s of waiting: giving up" >&2
    exit 1
  fi
  if [ -z "$said" ] || [ $((waited - said)) -ge 60 ]; then
    echo "waiting for the build Mac's simulators: in use by $(sed -n 3p "$LOCK/owner" 2>/dev/null || echo 'another run') (waited ${waited} s, gives up after $WAIT s)" >&2
    said=$waited
  fi
  if closed; then exit 0; fi
done
echo locked
# Held until stdin closes.
while :; do
  read -r _ || break
done
release
