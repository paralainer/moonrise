#!/usr/bin/env bash
# Build the Moonrise server image here and run it on a Docker host over ssh.
#
#   ./deploy.sh              build, keep the running image as moonrise-server:previous, load, start;
#                            if /health doesn't answer "ok", roll back to :previous at once
#   ./deploy.sh --rollback   run moonrise-server:previous (and the compose file it ran with) again
#
# The deployment is described in deploy/local/ (git-ignored, yours):
#   deploy.env               MOONRISE_DEPLOY_HOST=user@host (required), MOONRISE_DEPLOY_DIR (the compose
#                            project folder in the remote home, default moonrise-server),
#                            MOONRISE_DEPLOY_PORT (default 8097)
#   docker-compose.yml       the compose file to run (default: ./docker-compose.yml, the example)
#   builtin-keys.sh          optional, executable: prints the project's built-in keys as
#                            MOONRISE_BUILTIN_TMDB_KEY=… / MOONRISE_BUILTIN_OPENSUBTITLES_KEY=… lines
#                            (e.g. read from a password manager); they reach the build as a
#                            BuildKit secret and are compiled in (never written to the repo)
# Variables in the environment win over deploy.env.
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOCAL="$DIR/deploy/local"
if [ -f "$LOCAL/deploy.env" ]; then
    # Only the MOONRISE_DEPLOY_* lines, without overriding what the environment sets.
    while IFS='=' read -r k v; do
        case "$k" in MOONRISE_DEPLOY_*) [ -z "${!k:-}" ] && export "$k=$v" ;; esac
    done < <(grep -E '^MOONRISE_DEPLOY_[A-Z_]+=' "$LOCAL/deploy.env")
fi
HOST=${MOONRISE_DEPLOY_HOST:?set MOONRISE_DEPLOY_HOST=user@host (in deploy/local/deploy.env or the environment)}
RDIR=${MOONRISE_DEPLOY_DIR:-moonrise-server}
PORT=${MOONRISE_DEPLOY_PORT:-8097}
COMPOSE="$LOCAL/docker-compose.yml"
[ -f "$COMPOSE" ] || COMPOSE="$DIR/docker-compose.yml"

up() { ssh "$HOST" "cd ~/$RDIR && docker compose up -d --remove-orphans 2>&1 | tail -2"; }
health() {
    # Up to ~20 s for the server to start answering.
    ssh "$HOST" "for i in \$(seq 1 20); do r=\$(curl -s -m 2 localhost:$PORT/health || true); [ \"\$r\" = ok ] && { echo ok; exit 0; }; sleep 1; done; echo \"\${r:-no answer}\"; exit 1"
}
rollback() {
    echo "rolling back to moonrise-server:previous" >&2
    ssh "$HOST" "docker tag moonrise-server:previous moonrise-server:latest; cd ~/$RDIR && [ -f docker-compose.yml.previous ] && cp docker-compose.yml.previous docker-compose.yml || true"
    up
    health
}

if [ "${1:-}" = --rollback ]; then
    rollback
    exit
fi

SECRET=()
if [ -x "$LOCAL/builtin-keys.sh" ]; then
    KEYS=$(mktemp)
    chmod 600 "$KEYS"
    trap 'rm -f "$KEYS"' EXIT
    "$LOCAL/builtin-keys.sh" > "$KEYS"
    grep -qE '^MOONRISE_BUILTIN_[A-Z_]+=.' "$KEYS" || { echo "builtin-keys.sh printed no keys" >&2; exit 1; }
    SECRET=(--secret "id=builtin_keys,src=$KEYS")
fi
DOCKER_BUILDKIT=1 docker build -q "${SECRET[@]}" -t moonrise-server:latest "$DIR" >/dev/null
# Keep what runs now (image and compose file) as the rollback target, unless it is this very
# build, so a repeated deploy doesn't overwrite the last good one.
NEW=$(docker image inspect -f '{{.Id}}' moonrise-server:latest)
ssh "$HOST" "mkdir -p ~/$RDIR; cd ~/$RDIR; \
    cur=\$(docker image inspect -f '{{.Id}}' moonrise-server:latest 2>/dev/null || true); \
    if [ -n \"\$cur\" ] && [ \"\$cur\" != '$NEW' ]; then \
        docker tag moonrise-server:latest moonrise-server:previous; \
        [ -f docker-compose.yml ] && cp docker-compose.yml docker-compose.yml.previous; \
    fi; true"
docker save moonrise-server:latest | gzip -1 | ssh "$HOST" 'gunzip | docker load -q'
scp -q "$COMPOSE" "$HOST:$RDIR/docker-compose.yml"
up
if ! health; then
    rollback
    exit 1
fi
