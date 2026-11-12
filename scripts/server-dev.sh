#!/usr/bin/env bash
# The Moonrise server built from this checkout, in Docker (server/compose.dev.yml), for contributors.
# Needs only Docker (with compose) and curl.
#
#   scripts/server-dev.sh up     # build, start, wait until it answers, then print the address to
#                                # open and the setup code (make server-run)
#   scripts/server-dev.sh down   # stop and remove the container; the data stays (make server-stop)
#   scripts/server-dev.sh logs   # follow the server's log (make server-logs)
#   scripts/server-dev.sh url    # print the address and the setup code again
#
# The address printed is this machine's address on the local network (the one the apps and a
# browser on another device use), and localhost, not the container's: in a container with a
# published port the server's own log shows the container's internal address.
#
# COMPOSE_PROJECT_NAME  compose project (default moonrise-dev): another name runs a second server
# MOONRISE_DEV_PORT     port on this machine (default 8097)
# MOONRISE_DEV_IMAGE    image name (default moonrise-server:dev)
# MOONRISE_DEV_DATA     data folder (default .dev-data in the checkout; git-ignored)
# MOONRISE_DEV_MEDIA    a video folder, mounted read-only at /media (default .dev-media in the checkout)
# MOONRISE_DEV_HOST     the address to print (default: this machine's LAN address)
# HEALTH_WAIT           seconds to wait for the server to answer (default 120)
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
case "${1:-}" in
  -h|--help|"") awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }' "$0"; exit 0 ;;
esac

export COMPOSE_PROJECT_NAME=${COMPOSE_PROJECT_NAME:-moonrise-dev}
export MOONRISE_DEV_PORT=${MOONRISE_DEV_PORT:-8097}
export MOONRISE_DEV_IMAGE=${MOONRISE_DEV_IMAGE:-moonrise-server:dev}
export MOONRISE_DEV_DATA=${MOONRISE_DEV_DATA:-$ROOT/.dev-data}
export MOONRISE_DEV_MEDIA=${MOONRISE_DEV_MEDIA:-$ROOT/.dev-media}
# The server runs as you, so the data folder stays yours (and removable without sudo).
export MOONRISE_DEV_USER=${MOONRISE_DEV_USER:-$(id -u):$(id -g)}
COMPOSE=(docker compose -f "$ROOT/server/compose.dev.yml")

# host_address: this machine's IPv4 address on the local network, or nothing. The route lookup
# sends no packet.
host_address() {
  if [ -n "${MOONRISE_DEV_HOST:-}" ]; then printf '%s\n' "$MOONRISE_DEV_HOST"; return; fi
  local a=
  case "$(uname -s)" in
    Darwin) a=$(ipconfig getifaddr en0 2>/dev/null || ipconfig getifaddr en1 2>/dev/null || true) ;;
    *) a=$(ip -4 route get 192.0.2.1 2>/dev/null | sed -n 's/.* src \([0-9.]*\).*/\1/p' | head -1)
       [ -n "$a" ] || a=$(hostname -I 2>/dev/null | awk '{print $1}') ;;
  esac
  printf '%s\n' "$a"
}

# setup_code: the newest setup code in the log ("123 456"), or nothing once an account exists.
setup_code() {
  "${COMPOSE[@]}" logs --no-color server 2>/dev/null | sed 's/\x1b\[[0-9;]*m//g' \
    | sed -n 's/.*setup code: \([0-9]\{3\} [0-9]\{3\}\).*/\1/p' | tail -1
}

print_url() {
  local host code
  host=$(host_address)
  code=$(setup_code)
  echo
  echo "Moonrise server ($COMPOSE_PROJECT_NAME) is up:"
  [ -n "$host" ] && echo "  from other devices:  http://$host:$MOONRISE_DEV_PORT/"
  echo "  on this machine:     http://localhost:$MOONRISE_DEV_PORT/"
  if [ -n "$code" ]; then
    echo "  setup code:          $code   (enter it in the app, which finds the server or takes the address above)"
  else
    echo "  already set up (an account exists); sign in with it, or remove $MOONRISE_DEV_DATA to start over"
  fi
  echo "  data:                $MOONRISE_DEV_DATA"
  echo "  videos (read-only):  $MOONRISE_DEV_MEDIA, seen by the server as /media"
  echo "  stop it with \`make server-stop\` (or scripts/server-dev.sh down); the log: \`make server-logs\`"
}

up() {
  command -v curl >/dev/null || { echo "error: curl is needed to check the server is up" >&2; exit 1; }
  docker compose version >/dev/null 2>&1 || { echo "error: docker compose isn't available (Docker with the compose plugin is needed)" >&2; exit 1; }
  mkdir -p "$MOONRISE_DEV_DATA" "$MOONRISE_DEV_MEDIA"
  echo "==> building and starting the server from this checkout (the first build takes a few minutes)"
  "${COMPOSE[@]}" up -d --build
  local waited=0 limit=${HEALTH_WAIT:-120} id state
  id=$("${COMPOSE[@]}" ps -q server)
  until curl -fsS -o /dev/null "http://localhost:$MOONRISE_DEV_PORT/health" 2>/dev/null; do
    state=$(docker inspect -f '{{.State.Status}}' "$id" 2>/dev/null || echo gone)
    if [ "$state" != running ] || [ "$waited" -ge "$limit" ]; then
      echo "error: the server didn't answer on port $MOONRISE_DEV_PORT within $waited s (container: $state); its log:" >&2
      "${COMPOSE[@]}" logs --no-color --tail 30 server >&2 || true
      exit 1
    fi
    sleep 1; waited=$((waited + 1))
  done
  print_url
}

case "$1" in
  up) up ;;
  down) "${COMPOSE[@]}" down ;;
  logs) "${COMPOSE[@]}" logs -f server ;;
  url) print_url ;;
  *) echo "usage: $0 up|down|logs|url" >&2; exit 2 ;;
esac
