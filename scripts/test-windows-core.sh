#!/usr/bin/env bash
# Tests of the Windows app's shared rules (clients/windows/Moonrise.Core.Tests) in Docker, on any
# OS, against the shared cases in api/. Nothing is written into the checkout: the projects are
# copied to a scratch folder in the container.
#
#   scripts/test-windows-core.sh
#
# DOTNET_IMAGE  the .NET SDK image (default mcr.microsoft.com/dotnet/sdk:10.0)
# NUGET_VOLUME  Docker volume for the package cache (default moonrise-nuget)
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
case "${1:-}" in -h|--help) awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }' "$0"; exit 0 ;; esac
IMAGE=${DOTNET_IMAGE:-mcr.microsoft.com/dotnet/sdk:10.0}
VOLUME=${NUGET_VOLUME:-moonrise-nuget}
exec docker run --rm --name "moonrise-windows-tests-$$" \
  -v "$ROOT/clients/windows:/src/clients/windows:ro" -v "$ROOT/api:/src/api:ro" \
  -v "$VOLUME:/root/.nuget" -e DOTNET_CLI_TELEMETRY_OPTOUT=1 -e DOTNET_NOLOGO=1 \
  "$IMAGE" bash -c '
    mkdir -p /work/clients && cp -r /src/clients/windows /work/clients/ && cp -r /src/api /work/api
    cd /work/clients/windows/Moonrise.Core.Tests && dotnet test -c Release'
