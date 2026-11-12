#!/usr/bin/env bash
# Validates the API contract, api/openapi.yaml, and its examples (api/examples/*.json, checked
# against their schemas) with Redocly CLI, version-pinned, in Docker (api/redocly.yaml: the rules).
# That every server route is in it is a server test (server/src/api_contract_tests.rs).
#
#   scripts/api-lint.sh
#
# REDOCLY_IMAGE   the image (default redocly/cli:2.76.0)
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
case "${1:-}" in
  -h|--help) awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }' "$0"; exit 0 ;;
  "") ;;
  *) echo "usage: $0" >&2; exit 2 ;;
esac
IMAGE=${REDOCLY_IMAGE:-redocly/cli:2.76.0}
[ -f "$ROOT/api/openapi.yaml" ] || { echo "no api/openapi.yaml" >&2; exit 1; }
exec docker run --rm -v "$ROOT/api:/spec:ro" -w /spec -e REDOCLY_TELEMETRY=off -e REDOCLY_SUPPRESS_UPDATE_NOTICE=true \
  "$IMAGE" lint --config redocly.yaml openapi.yaml
