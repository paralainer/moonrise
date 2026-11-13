#!/usr/bin/env bash
# Build the Android client in Docker: nothing but Docker is needed on the machine.
#
#   clients/android/build.sh                 # debug APK: tv/build/outputs/apk/debug/tv-debug.apk
#   clients/android/build.sh phone           # the phone app's debug APK: phone/build/outputs/apk/debug/phone-debug.apk
#   clients/android/build.sh test            # the API client's unit tests (:core:test), and the shared
#                                            # module built without the Tailscale library
#   clients/android/build.sh release         # signed release APKs, one per CPU type and a universal one:
#                                            # tv/build/outputs/apk/release/tv-<abi>-release.apk
#                                            # (signing: MOONRISE_ANDROID_SIGNING=<signing.properties>, see README)
#   clients/android/build.sh phone-release   # the same for the phone app (phone/.../phone-<abi>-release.apk)
#   clients/android/build.sh <gradle args>   # anything else, e.g. ":tv:assembleRelease"
#   clients/android/build.sh --shell         # a shell in the build container
#
# The image (moonrise-android-build, rebuilt when docker/ changes) holds JDK 17 and the Android command-line tools. The SDK
# packages and Gradle's caches live in named volumes, kept between builds:
#   moonrise-android-sdk     Android SDK (platform, build tools, platform-tools)
#   moonrise-android-gradle  Gradle distribution and dependency caches, and the debug signing
#                            key (android-user/debug.keystore), so debug builds update in place
# Remove them with `docker volume rm moonrise-android-sdk moonrise-android-gradle` to start over.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# The API contract (api/ at the repository's root): the core tests decode its example answers.
API_DIR="$(cd "$HERE/../../api" && pwd)"
IMAGE="moonrise-android-build:1"
SDK_VOLUME="moonrise-android-sdk"
GRADLE_VOLUME="moonrise-android-gradle"
# Kept in step with tv/build.gradle.kts (compileSdk) and AGP's default build tools.
SDK_PACKAGES=("platform-tools" "platforms;android-35" "build-tools;35.0.0")

# The image is rebuilt when docker/ changes: it carries a hash of that folder as a label.
recipe="$(cd "$HERE/docker" && find . -type f -print0 | sort -z | xargs -0 sha256sum | sha256sum | cut -c1-16)"
if [ "$(docker image inspect -f '{{index .Config.Labels "app.moonrise.recipe"}}' "$IMAGE" 2>/dev/null)" != "$recipe" ]; then
    echo "==> building $IMAGE"
    docker build --label "app.moonrise.recipe=$recipe" -t "$IMAGE" "$HERE/docker"
fi

uid="$(id -u)"; gid="$(id -g)"
for v in "$SDK_VOLUME" "$GRADLE_VOLUME"; do
    if ! docker volume inspect "$v" >/dev/null 2>&1; then
        docker volume create "$v" >/dev/null
        # Owned by the caller, so the build runs as them and leaves no root-owned files.
        docker run --rm -v "$v:/v" "$IMAGE" chown "$uid:$gid" /v
    fi
done

# Release signing: the properties file (and the keystore beside it) is mounted read-only at /signing.
signing=()
if [ -n "${MOONRISE_ANDROID_SIGNING:-}" ]; then
    [ -f "$MOONRISE_ANDROID_SIGNING" ] || { echo "MOONRISE_ANDROID_SIGNING: no such file: $MOONRISE_ANDROID_SIGNING" >&2; exit 1; }
    sdir="$(cd "$(dirname "$MOONRISE_ANDROID_SIGNING")" && pwd)"
    signing=(-v "$sdir:/signing:ro" -e "MOONRISE_ANDROID_SIGNING=/signing/$(basename "$MOONRISE_ANDROID_SIGNING")")
fi

run() {
    local tty=()
    [ -t 0 ] && [ -t 1 ] && tty=(-it)
    docker run --rm "${tty[@]}" --user "$uid:$gid" \
        -e HOME=/tmp -e ANDROID_USER_HOME=/gradle/android-user -e GRADLE_OPTS="-Dorg.gradle.daemon=false -Duser.home=/tmp" \
        -v "$SDK_VOLUME:/opt/android-sdk" -v "$GRADLE_VOLUME:/gradle" "${signing[@]}" \
        -v "$API_DIR:/api:ro" -e MOONRISE_API_EXAMPLES=/api/examples -e MOONRISE_SEGMENT_RULES=/api/segment-rules.json -e MOONRISE_RESUME_RULES=/api/resume-rules.json -e MOONRISE_PLAYBACK_PROBLEM_RULES=/api/playback-problem-rules.json \
        -v "$HERE:/src" -w /src "$IMAGE" "$@"
}

# SDK packages, once (licences accepted for the build).
if ! run test -d "/opt/android-sdk/platforms/android-35"; then
    echo "==> installing Android SDK packages into $SDK_VOLUME"
    run bash -c "set -o pipefail; yes | sdkmanager --sdk_root=/opt/android-sdk --licenses >/dev/null; sdkmanager --sdk_root=/opt/android-sdk $(printf '"%s" ' "${SDK_PACKAGES[@]}")"
fi

case "${1:-}" in
    "") run ./gradlew --no-daemon :tv:assembleDebug
        echo "==> $HERE/tv/build/outputs/apk/debug/tv-debug.apk" ;;
    phone) run ./gradlew --no-daemon :phone:assembleDebug
        echo "==> $HERE/phone/build/outputs/apk/debug/phone-debug.apk" ;;
    # The apps build without the Tailscale library too (src/notailscale): checked whether or not
    # vendor/moonrise-tailscale.aar is here.
    test) run ./gradlew --no-daemon :core:test
        run ./gradlew --no-daemon -Pmoonrise.tailscale=false :shared:compileDebugKotlin ;;
    release|phone-release)
        app=tv; [ "$1" = phone-release ] && app=phone
        rm -f "$HERE/$app/build/outputs/apk/release/"*.apk
        run ./gradlew --no-daemon ":$app:assembleRelease"
        dir="$app/build/outputs/apk/release"
        # One APK per CPU type plus a universal one (the ABI splits); not signed (no signing set
        # up), Gradle names them *-unsigned.apk.
        ls "$HERE/$dir/$app-universal-release.apk" >/dev/null 2>&1 || { echo "==> no signed APK: set MOONRISE_ANDROID_SIGNING (see clients/android/README.md)" >&2; exit 1; }
        run /opt/android-sdk/build-tools/35.0.0/apksigner verify --print-certs "$dir/$app-universal-release.apk" | grep -E "Signer #1 certificate (DN|SHA-256)"
        for apk in "$HERE/$dir/"*-release.apk; do echo "==> $apk"; done ;;
    --shell) run bash ;;
    *) run ./gradlew --no-daemon "$@" ;;
esac
