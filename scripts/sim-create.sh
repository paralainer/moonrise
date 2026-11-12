#!/usr/bin/env bash
# Find or create a simulator on the build Mac for sim.sh and tvsim.sh, and print its UDID.
#
#   scripts/sim-create.sh [phone]          # "Moonrise iPhone": the newest iOS, an iPhone 17 (or 16, 15)
#   scripts/sim-create.sh tv               # "Moonrise Apple TV": the newest tvOS, an Apple TV 4K
#   scripts/sim-create.sh --delete [phone|tv]   # delete that simulator again
#
# The same name is found again next time, so running it twice makes one simulator. sim.sh and
# tvsim.sh call it when no simulator is set (SIM, or SIM_PHONE / SIM_TV in scripts/mac.env); put the
# UDID it prints in mac.env to use that one from then on. SIM_NAME picks another name.
# On a Mac itself it runs locally (MAC=local, scripts/common.sh).
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/common.sh"
DELETE=
[ "${1:-}" = --delete ] && { DELETE=1; shift; }
case "${1:-phone}" in
  phone) NAME=${SIM_NAME:-Moonrise iPhone}; OS=iOS ;;
  tv) NAME=${SIM_NAME:-Moonrise Apple TV}; OS=tvOS ;;
  *) echo "usage: $0 [--delete] [phone|tv]" >&2; exit 2 ;;
esac
case "$NAME" in *\'*) echo "error: SIM_NAME can't contain a quote" >&2; exit 2 ;; esac
need_mac_reachable

# On the Mac: the UDID of an available simulator with exactly this name (the first, if several).
FIND="xcrun simctl list devices available | sed -n 's/^ *$NAME (\\([0-9A-F-]\\{36\\}\\)).*/\\1/p' | head -1"
if [ -n "$DELETE" ]; then
  ssh "$MAC" "$MAC_PATH
u=\$($FIND)
[ -n \"\$u\" ] || { echo 'no simulator named \"$NAME\"' >&2; exit 0; }
xcrun simctl shutdown \$u >/dev/null 2>&1; xcrun simctl delete \$u && echo \"deleted $NAME (\$u)\" >&2"
  exit
fi
u=$(ssh "$MAC" "$MAC_PATH
$FIND")
if [ -z "$u" ]; then
  # scripts/sim-new.sh picks the newest installed runtime and a device type it takes.
  u=$(ssh "$MAC" "$MAC_PATH; bash -s $OS '$NAME'" < "$(dirname "${BASH_SOURCE[0]}")/sim-new.sh") || exit 1
fi
echo "$u"
