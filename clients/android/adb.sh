#!/usr/bin/env bash
# adb from the build image's SDK volume (no host install), on the host network so it reaches
# TVs on the LAN. Uses this machine's adb key (~/.android/adbkey), the one a TV has allowed.
#
#   clients/android/adb.sh connect 192.168.1.50:5555
#   clients/android/adb.sh -s 192.168.1.50:5555 shell ...
#   clients/android/adb.sh stop          # stop the adb server container
#
# The adb server runs in the container `moonrise-android-adb` on port 5038, so it doesn't disturb
# another adb server on the default 5037. ADB_PORT=<port> picks another port: when an adb server
# already answers there (say the computer's own one on 5037, the one that sees devices on USB),
# commands go to that server, from a throwaway container, and nothing is started; otherwise the
# container's server starts on that port. Commands run in a container that sees your home
# directory read-only (for `install` and the like); output goes to stdout.
set -euo pipefail
IMAGE="moonrise-android-build:1"
NAME="moonrise-android-adb"
PORT="${ADB_PORT:-5038}"
KEYS="${ADB_KEYS:-$HOME/.android}"

if [ "${1:-}" = "stop" ]; then
    docker rm -f "$NAME" >/dev/null 2>&1 || true
    exit 0
fi

tty=()
[ -t 0 ] && [ -t 1 ] && tty=(-t)
wd=/
case "$PWD" in "$HOME"|"$HOME"/*) wd="$PWD" ;; esac

ours_running() {
    [ "$(docker inspect -f '{{.State.Running}} {{index .Config.Labels "app.moonrise.adb-port"}}' "$NAME" 2>/dev/null)" = "true $PORT" ]
}
listening() { (exec 3<>"/dev/tcp/127.0.0.1/$PORT") 2>/dev/null; }

if ! ours_running && listening; then
    # Another adb server on this port: a client only, which starts no server of its own.
    exec docker run --rm -i "${tty[@]}" --network host --user "$(id -u):$(id -g)" -e HOME=/tmp \
        -v moonrise-android-sdk:/opt/android-sdk:ro -v "$HOME:$HOME:ro" -w "$wd" \
        "$IMAGE" adb -P "$PORT" "$@"
fi

if ! ours_running; then
    docker rm -f "$NAME" >/dev/null 2>&1 || true
    docker run -d --name "$NAME" --network host --user "$(id -u):$(id -g)" \
        --label "app.moonrise.adb-port=$PORT" \
        -e HOME=/home/adb -e ANDROID_USER_HOME=/home/adb/.android -e ANDROID_ADB_SERVER_PORT="$PORT" \
        -v moonrise-android-sdk:/opt/android-sdk:ro -v "$KEYS:/home/adb/.android" \
        -v "$HOME:$HOME:ro" \
        "$IMAGE" adb -P "$PORT" server nodaemon >/dev/null
    sleep 1
fi

exec docker exec -i "${tty[@]}" -w "$wd" "$NAME" adb -P "$PORT" "$@"
