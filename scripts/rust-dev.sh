#!/usr/bin/env bash
# Run a command in the server's dev container: the Rust toolchain pinned in server/rust-toolchain.toml,
# with clippy and rustfmt (server/docker/dev.Dockerfile). The image is built the first time it's
# needed, and again when the Dockerfile or the pinned version changes; nothing else is installed.
#
#   scripts/rust-dev.sh cargo test --locked           # in server/
#   scripts/rust-dev.sh cargo clippy --all-targets -- -D warnings
#   scripts/rust-dev.sh --image                        # build if needed, print the image's name
#
# RUST_DEV_IMAGE   the image (default moonrise-rust-dev:<version>)
# CARGO_VOLUME     Docker volume for cargo's registry cache (default moonrise-cargo)
# CARGO_TARGET_VOLUME  Docker volume for the build output (default moonrise-test-target)
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
case "${1:-}" in
  -h|--help|"") awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }' "$0"; exit 0 ;;
esac
TOOLCHAIN="$ROOT/server/rust-toolchain.toml"
DOCKERFILE="$ROOT/server/docker/dev.Dockerfile"
VERSION=$(sed -n 's/^channel *= *"\(.*\)".*/\1/p' "$TOOLCHAIN" | head -1)
[ -n "$VERSION" ] || { echo "no channel in $TOOLCHAIN" >&2; exit 1; }
IMAGE=${RUST_DEV_IMAGE:-moonrise-rust-dev:$VERSION}
CARGO_VOLUME=${CARGO_VOLUME:-moonrise-cargo}
CARGO_TARGET_VOLUME=${CARGO_TARGET_VOLUME:-moonrise-test-target}

# The image carries a hash of its recipe (the Dockerfile and the version), so an edit rebuilds it.
if command -v sha256sum >/dev/null; then sum=(sha256sum); else sum=(shasum -a 256); fi
recipe=$( { cat "$DOCKERFILE"; echo "$VERSION"; } | "${sum[@]}" | cut -c1-16)
have=$(docker image inspect -f '{{ index .Config.Labels "moonrise.dev-recipe" }}' "$IMAGE" 2>/dev/null || true)
if [ "$have" != "$recipe" ]; then
  echo "==> building the dev image $IMAGE (Rust $VERSION, first use or changed recipe)" >&2
  docker build -q -f "$DOCKERFILE" --build-arg "RUST_VERSION=$VERSION" \
    --label "moonrise.dev-recipe=$recipe" -t "$IMAGE" "$ROOT/server/docker" >/dev/null
fi

if [ "$1" = --image ]; then
  printf '%s\n' "$IMAGE"
  exit 0
fi
# The API contract (api/) beside the server, read-only: a server test checks the router against it.
exec docker run --rm -v "$CARGO_VOLUME:/usr/local/cargo/registry" -v "$CARGO_TARGET_VOLUME:/target" \
  -e CARGO_TARGET_DIR=/target -v "$ROOT/server:/src" -v "$ROOT/api:/api:ro" -w /src "$IMAGE" "$@"
