#!/bin/bash
# Removes the Moonrise server service and program from this Mac.
#
#   uninstall.sh          # stops it, removes the service and ~/Applications/Moonrise Server; keeps
#                         # the data (accounts, watch history, settings) for a later install
#   uninstall.sh --all    # also deletes the data, the cache and the logs
#
# The ffmpeg it used stays if it was Homebrew's (brew uninstall ffmpeg removes that).
set -euo pipefail

LABEL=org.moonrise.server
APP_DIR="$HOME/Applications/Moonrise Server"
DATA_DIR="$HOME/Library/Application Support/Moonrise Server"
CACHE_DIR="$HOME/Library/Caches/Moonrise Server"
LOG_DIR="$HOME/Library/Logs/Moonrise Server"
ALL=0
[ "${1:-}" = --all ] && ALL=1

launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null && echo "==> stopped the LaunchAgent" || true
launchctl bootout "user/$(id -u)/$LABEL" 2>/dev/null && echo "==> stopped the background-session agent" || true
rm -f "$HOME/Library/LaunchAgents/$LABEL.plist"
if [ -f "/Library/LaunchDaemons/$LABEL.plist" ]; then
  echo "==> removing the LaunchDaemon (asks for your password)"
  sudo launchctl bootout "system/$LABEL" 2>/dev/null || true
  sudo rm -f "/Library/LaunchDaemons/$LABEL.plist"
fi
rm -rf "$APP_DIR"
echo "==> removed $APP_DIR"
if [ "$ALL" = 1 ]; then
  rm -rf "$DATA_DIR" "$CACHE_DIR" "$LOG_DIR"
  echo "==> deleted the data, cache and logs"
else
  echo "==> kept the data in $DATA_DIR (uninstall.sh --all deletes it)"
  rm -rf "$CACHE_DIR"
fi
