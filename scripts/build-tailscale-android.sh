#!/usr/bin/env bash
# Build the Android apps' Tailscale library into clients/android/vendor/moonrise-tailscale.aar:
# clients/android/tailscale (a small Go package over tailscale.com/tsnet, pinned in its go.mod)
# bound to Kotlin with gomobile, for arm, arm64 and x86_64 (emulators). Only Docker is needed.
# Not committed; rerun after a fresh checkout or to update Tailscale. Without it the apps build
# without remote access over Tailscale (see clients/android/README.md).
#
#   scripts/build-tailscale-android.sh          # build the .aar
#   scripts/build-tailscale-android.sh test     # the Go package's tests only
#   TS_ANDROID_TARGETS=android/386,... scripts/build-tailscale-android.sh   # other ABIs (an x86 emulator)
#
# Image: moonrise-tailscale-android-build (the Android build image plus Go, rebuilt when
# clients/android/tailscale/docker changes). The NDK and Go's caches live in the named volume
# moonrise-tailscale-android (TS_ANDROID_CACHE=<dir> to use a folder instead); the Android SDK
# is the Android build's volume, read-only. Remove the volume to start over.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ANDROID="$HERE/clients/android"
IMAGE="moonrise-tailscale-android-build:1"
CACHE="${TS_ANDROID_CACHE:-moonrise-tailscale-android}"
NDK="27.2.12479018"
# Phones and TVs (arm, arm64) and x86_64 emulators.
TARGETS="${TS_ANDROID_TARGETS:-android/arm,android/arm64,android/amd64}"

# The Android build image and its SDK volume first (they are made by build.sh).
docker image inspect moonrise-android-build:1 >/dev/null 2>&1 && docker volume inspect moonrise-android-sdk >/dev/null 2>&1 \
    || "$ANDROID/build.sh" help >/dev/null
recipe="$(cd "$ANDROID/tailscale/docker" && find . -type f -print0 | sort -z | xargs -0 sha256sum | sha256sum | cut -c1-16)"
if [ "$(docker image inspect -f '{{index .Config.Labels "app.moonrise.recipe"}}' "$IMAGE" 2>/dev/null)" != "$recipe" ]; then
    echo "==> building $IMAGE"
    docker build --label "app.moonrise.recipe=$recipe" -t "$IMAGE" "$ANDROID/tailscale/docker"
fi

uid="$(id -u)"; gid="$(id -g)"
if [[ "$CACHE" == /* ]]; then mkdir -p "$CACHE"
elif ! docker volume inspect "$CACHE" >/dev/null 2>&1; then
    docker volume create "$CACHE" >/dev/null
    docker run --rm -v "$CACHE:/cache" "$IMAGE" chown "$uid:$gid" /cache
fi

run() {
    docker run --rm --user "$uid:$gid" -e HOME=/tmp \
        -e GOMODCACHE=/cache/gomod -e GOCACHE=/cache/gocache -e GOPATH=/cache/gopath \
        -e ANDROID_NDK_HOME="/cache/sdk/ndk/$NDK" \
        -v "$CACHE:/cache" -v moonrise-android-sdk:/opt/android-sdk:ro \
        -v "$ANDROID/tailscale:/src" -v "$ANDROID/vendor:/out" -w /src "$IMAGE" bash -c "$1"
}
mkdir -p "$ANDROID/vendor"

case "${1:-}" in
    test) run 'go vet ./... && go test ./...'; exit ;;
    "") ;;
    *) echo "usage: $0 [test]" >&2; exit 2 ;;
esac

run "set -euo pipefail
[ -d /cache/sdk/ndk/$NDK ] || { echo '==> installing NDK $NDK'; { yes || true; } | sdkmanager --sdk_root=/cache/sdk --licenses >/dev/null || true; sdkmanager --sdk_root=/cache/sdk 'ndk;$NDK' | tail -1; }
go install golang.org/x/mobile/cmd/gomobile golang.org/x/mobile/cmd/gobind
export PATH=/cache/gopath/bin:\$PATH
go test ./...
gomobile bind -target=$TARGETS -androidapi 26 -javapkg app.moonrise.tsnet \
    -trimpath -ldflags='-s -w' -o /out/moonrise-tailscale.aar .
rm -f /out/moonrise-tailscale-sources.jar
ls -la /out"
echo "==> $ANDROID/vendor/moonrise-tailscale.aar"
