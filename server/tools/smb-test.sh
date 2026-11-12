#!/usr/bin/env bash
# The SMB integration test: a Samba server in Docker (ghcr.io/servercontainers/samba) serving a
# fixture folder (a short Matroska film with an embedded and a sidecar subtitle track, an MP4,
# a binary file), then `cargo test -- --ignored smb_` (src/storage/smb_it.rs) in a Rust container on the
# host network. Nothing is installed on the host; everything is removed afterwards.
#
#   tools/smb-test.sh            run it
#   KEEP=1 tools/smb-test.sh     leave the Samba container running (127.0.0.1:1445, user moonrise)
#
# Needs: docker, the moonrise-server image (for ffmpeg, to make the fixture: ./deploy.sh builds it,
# or `docker build -t moonrise-server:latest .`), and the dev image (RUST_IMAGE; by default
# scripts/rust-dev.sh builds it on first use: the toolchain in rust-toolchain.toml). Cargo caches: volumes CARGO_VOLUME (registry, moonrise-cargo) and TARGET_VOLUME
# (moonrise-target-smbtest). SMB_NAME names the containers (moonrise-smbtest), PORT the local port
# (1445): set both for a run beside another one. scripts/test-all.sh smb runs this.
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUST_IMAGE=${RUST_IMAGE:-$("$DIR/../scripts/rust-dev.sh" --image)}
FFMPEG_IMAGE=${FFMPEG_IMAGE:-moonrise-server:latest}
SAMBA_IMAGE=${SAMBA_IMAGE:-ghcr.io/servercontainers/samba:latest}
PORT=${PORT:-1445}
NAME=${SMB_NAME:-moonrise-smbtest}
CARGO_VOLUME=${CARGO_VOLUME:-moonrise-cargo}
PASS="pw-$(head -c 8 /dev/urandom | od -An -tx1 | tr -d ' \n')"
WORK=$(mktemp -d)
cleanup() {
    [ -n "${KEEP:-}" ] || docker rm -f "$NAME" >/dev/null 2>&1 || true
    docker run --rm -v "$WORK:/w" alpine:3 rm -rf /w/media /w/public >/dev/null 2>&1 || true
    rm -rf "$WORK"
}
trap cleanup EXIT

echo "== fixture"
mkdir -p "$WORK/media/movies/Test Film (2010)" "$WORK/media/movies/Other (2011)" "$WORK/public"
cat > "$WORK/sub.srt" <<'SRT'
1
00:00:01,000 --> 00:00:03,000
Hello from the share.

2
00:01:00,000 --> 00:01:02,500
A minute in.
SRT
docker run --rm --user "$(id -u):$(id -g)" -v "$WORK:/w" --entrypoint sh "$FFMPEG_IMAGE" -c '
  set -e
  ffmpeg -v error -f lavfi -i testsrc=duration=150:size=320x240:rate=25 -f lavfi -i sine=duration=150 -i /w/sub.srt \
    -map 0 -map 1 -map 2 -c:v libx264 -g 50 -preset ultrafast -c:a aac -c:s srt -metadata:s:s:0 language=eng \
    "/w/media/movies/Test Film (2010)/Test Film (2010).mkv"
  cp /w/sub.srt "/w/media/movies/Test Film (2010)/Test Film (2010).en.srt"
  ffmpeg -v error -f lavfi -i testsrc=duration=130:size=160x120:rate=25 -c:v libx264 -preset ultrafast -movflags +faststart \
    "/w/media/movies/Other (2011)/Other (2011).mp4"
  head -c 12500000 /dev/urandom > /w/media/movies/blob.bin
  echo hi > /w/public/readme.txt'
chmod -R a+rX "$WORK/media" "$WORK/public"

echo "== samba on 127.0.0.1:$PORT"
docker rm -f "$NAME" >/dev/null 2>&1 || true
docker run -d --name "$NAME" -p "127.0.0.1:$PORT:445" \
    -e ACCOUNT_moonrise="$PASS" -e UID_moonrise="$(id -u)" \
    -e AVAHI_DISABLE=1 -e WSDD2_DISABLE=1 -e NETBIOS_DISABLE=1 \
    -e SAMBA_VOLUME_CONFIG_media="[media]; path=/shares/media; valid users = moonrise; guest ok = no; read only = yes; browseable = yes" \
    -e SAMBA_VOLUME_CONFIG_public="[public]; path=/shares/public; guest ok = yes; read only = yes; browseable = yes" \
    -v "$WORK/media:/shares/media:ro" -v "$WORK/public:/shares/public:ro" \
    "$SAMBA_IMAGE" >/dev/null
# Up when smbd answers inside the container (the published port accepts earlier).
for i in $(seq 1 60); do
    docker exec "$NAME" smbclient -L 127.0.0.1 -N >/dev/null 2>&1 && break
    sleep 1
done

echo "== cargo test -- --ignored smb_against"
docker run --rm --network host \
    -v "$CARGO_VOLUME:/usr/local/cargo/registry" -v "${TARGET_VOLUME:-moonrise-target-smbtest}:/target" -e CARGO_TARGET_DIR=/target \
    -v "$DIR:/src:ro" -v "$WORK/media:/fixture:ro" -w /src \
    -e MOONRISE_SMB_TEST_HOST="127.0.0.1:$PORT" -e MOONRISE_SMB_TEST_USER=moonrise -e MOONRISE_SMB_TEST_PASS="$PASS" -e MOONRISE_SMB_TEST_FIXTURE=/fixture \
    "$RUST_IMAGE" bash -c 'set -o pipefail; cargo test -- --ignored smb_against --nocapture 2>&1 | grep -v "^ *Compiling"'

echo "== cargo test -- --ignored smb_stopped_reader"
docker run --rm --network host \
    -v "$CARGO_VOLUME:/usr/local/cargo/registry" -v "${TARGET_VOLUME:-moonrise-target-smbtest}:/target" -e CARGO_TARGET_DIR=/target \
    -v "$DIR:/src:ro" -v "$WORK/media:/fixture:ro" -w /src \
    -e MOONRISE_SMB_TEST_HOST="127.0.0.1:$PORT" -e MOONRISE_SMB_TEST_USER=moonrise -e MOONRISE_SMB_TEST_PASS="$PASS" -e MOONRISE_SMB_TEST_FIXTURE=/fixture \
    "$RUST_IMAGE" bash -c 'set -o pipefail; cargo test -- --ignored smb_stopped_reader --nocapture 2>&1 | grep -v "^ *Compiling"'

echo "== cargo test -- --ignored smb_reconnects (Samba restarted under it)"
mkdir -p "$WORK/ctl"
docker run -d --name "$NAME-run" --network host \
    -v "$CARGO_VOLUME:/usr/local/cargo/registry" -v "${TARGET_VOLUME:-moonrise-target-smbtest}:/target" -e CARGO_TARGET_DIR=/target \
    -v "$DIR:/src:ro" -v "$WORK/media:/fixture:ro" -v "$WORK/ctl:/ctl" -w /src \
    -e MOONRISE_SMB_TEST_HOST="127.0.0.1:$PORT" -e MOONRISE_SMB_TEST_USER=moonrise -e MOONRISE_SMB_TEST_PASS="$PASS" -e MOONRISE_SMB_TEST_FIXTURE=/fixture -e MOONRISE_SMB_TEST_CTL=/ctl \
    "$RUST_IMAGE" cargo test -- --ignored smb_reconnects --nocapture >/dev/null
for i in $(seq 1 300); do [ -f "$WORK/ctl/ready" ] && break; sleep 1; done
docker restart -t 1 "$NAME" >/dev/null
touch "$WORK/ctl/restarted"
code=$(docker wait "$NAME-run")
docker logs "$NAME-run" 2>&1 | grep -E -A3 "reads back|panicked" || true
docker rm -f "$NAME-run" >/dev/null
docker run --rm -v "$WORK:/w" alpine:3 rm -rf /w/ctl >/dev/null 2>&1 || true
[ "$code" = 0 ]
if [ -n "${KEEP:-}" ]; then
    echo "samba left running: smb://127.0.0.1:$PORT/media (user moonrise, password $PASS), smb://127.0.0.1:$PORT/public (guest)"
fi
