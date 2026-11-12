#!/usr/bin/env bash
# Create a simulator and print its UDID (one line, nothing else on stdout). Runs on the Mac:
# scripts/test-apple.sh calls it in its copy of the tree, scripts/sim-create.sh sends it over ssh.
#
#   scripts/sim-new.sh iOS  NAME     # the newest installed iOS runtime, an iPhone
#   scripts/sim-new.sh tvOS NAME     # the newest installed tvOS runtime, an Apple TV
#
# The runtime and device type are what this Mac's Xcode has installed (a fixed guess broke
# when the build Mac moved to another Xcode): the newest available runtime of the OS, and the first
# of the preferred device types that runtime takes, else any iPhone / Apple TV type it takes.
# Exits 1 with simctl's own message when none works; when a create exits 0 but prints no UDID,
# the simulator it made is deleted and it exits 1.
set -uo pipefail
OS=${1:?usage: sim-new.sh iOS|tvOS NAME}
NAME=${2:?usage: sim-new.sh iOS|tvOS NAME}
case "$OS" in
  iOS)  PREFER=('iPhone 17' 'iPhone 16' 'iPhone 16e' 'iPhone 15'); FAMILY=iPhone ;;
  tvOS) PREFER=('Apple TV 4K (3rd generation)' 'Apple TV 4K (2nd generation)' 'Apple TV'); FAMILY='Apple TV' ;;
  *) echo "usage: sim-new.sh iOS|tvOS NAME" >&2; exit 2 ;;
esac
UUID_RE='[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}'

# Runtimes: identifiers like com.apple.CoreSimulator.SimRuntime.iOS-26-3, newest version last.
runtimes=$(xcrun simctl list runtimes available 2>&1) || { echo "error: xcrun simctl list runtimes: $runtimes" >&2; exit 1; }
runtime=$(printf '%s\n' "$runtimes" | grep -E "^$OS " \
  | grep -oE "com\.apple\.CoreSimulator\.SimRuntime\.$OS-[0-9]+(-[0-9]+)*" \
  | awk -F"$OS-" '{ v = $2; gsub("-", ".", v); print v " " $0 }' | sort -V | tail -1 | cut -d' ' -f2)
if [ -z "$runtime" ]; then
  echo "error: no $OS simulator runtime installed on the Mac (Xcode → Settings → Components); simctl lists:" >&2
  printf '%s\n' "$runtimes" | sed 's/^/  /' >&2
  exit 1
fi

# Device types: the preferred names, then every other type of the family, newest (last listed) first.
types=$(xcrun simctl list devicetypes 2>/dev/null | sed -n "s/^\($FAMILY.*\) (com\.apple\.CoreSimulator\.SimDeviceType\.[^)]*)$/\1/p")
candidates=("${PREFER[@]}")
while IFS= read -r t; do [ -n "$t" ] && candidates+=("$t"); done < <(printf '%s\n' "$types" | sed '1!G;h;$!d')

# sims_named: the UDIDs of the simulators named exactly $NAME, one per line.
sims_named() {
  local l u
  xcrun simctl list devices 2>/dev/null | while IFS= read -r l; do
    l=${l#"${l%%[! ]*}"}
    case "$l" in
      "$NAME ("*) u=${l#"$NAME ("}; u=${u%%)*}; [[ $u =~ ^$UUID_RE$ ]] && echo "$u" ;;
    esac
  done
}

last=
for type in "${candidates[@]}"; do
  before=$(sims_named)
  out=$(xcrun simctl create "$NAME" "$type" "$runtime" 2>&1) || { last=$out; continue; }
  udid=$(printf '%s\n' "$out" | grep -oE "$UUID_RE" | tail -1)
  if [ -n "$udid" ]; then
    echo "created simulator $NAME: $type, ${runtime##*.} ($udid)" >&2
    echo "$udid"
    exit 0
  fi
  # Created, but no UDID to give back: delete what this create made, found by its name
  # (only the simulators with that name that weren't there before it), rather than leave it behind.
  echo "error: simctl create exited 0 but printed no UDID: $out" >&2
  made=0
  for u in $(sims_named); do
    grep -qxF "$u" <<<"$before" && continue
    made=1
    if xcrun simctl delete "$u" >/dev/null 2>&1; then
      echo "deleted the simulator it made: $NAME ($u)" >&2
    else
      echo "error: could not delete the simulator it made: $NAME ($u); delete it with: xcrun simctl delete $u" >&2
    fi
  done
  [ "$made" = 1 ] || echo "no new simulator named $NAME is listed, so nothing was left behind" >&2
  exit 1
done
echo "error: could not create a $OS simulator on ${runtime##*.} (tried: ${candidates[*]}): $last" >&2
exit 1
