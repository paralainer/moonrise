#!/usr/bin/env bash
# scripts/api-lint.sh validates api/openapi.yaml with the pinned Redocly CLI image, in
# Docker, and fails when it does. docker is a stub that notes its call and answers with $LINT_RC.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
C=$(fresh_clone)
mkdir -p "$SCRATCH/bin" "$C/api"
cat > "$SCRATCH/bin/docker" <<STUB
#!/bin/sh
echo "docker \$*" >> "$SCRATCH/calls"
exit \${LINT_RC:-0}
STUB
chmod +x "$SCRATCH/bin/docker"
run() { : > "$SCRATCH/calls"; env -u REDOCLY_IMAGE PATH="$SCRATCH/bin:$PATH" "$@" 2>&1; }

out=$(run bash "$C/scripts/api-lint.sh" --help); rc=$?
expect_status 0 $rc "api-lint.sh --help" "$out"
expect_in "REDOCLY_IMAGE" "$out" "--help names the image setting"
out=$(run bash "$C/scripts/api-lint.sh" extra); rc=$?
expect_status 2 $rc "api-lint.sh with an argument" "$out"

out=$(run bash "$C/scripts/api-lint.sh"); rc=$?
expect_status 1 $rc "api-lint.sh without api/openapi.yaml" "$out"
expect_in "no api/openapi.yaml" "$out" "it says what is missing"

touch "$C/api/openapi.yaml"
out=$(run bash "$C/scripts/api-lint.sh"); rc=$?
expect_status 0 $rc "api-lint.sh, valid" "$out"
calls=$(cat "$SCRATCH/calls")
expect_in "redocly/cli:2.76.0 lint --config redocly.yaml openapi.yaml" "$calls" "the pinned image lints the spec"
expect_in "-v $C/api:/spec:ro" "$calls" "api/ is mounted read-only"
out=$(run env LINT_RC=1 bash "$C/scripts/api-lint.sh"); rc=$?
expect_status 1 $rc "api-lint.sh, invalid" "$out"
out=$(run env REDOCLY_IMAGE=redocly/cli:9.9.9 bash "$C/scripts/api-lint.sh"); rc=$?
expect_in "redocly/cli:9.9.9 lint" "$(cat "$SCRATCH/calls")" "REDOCLY_IMAGE overrides the image"

# Pinned to a version, never a floating tag.
expect_in "redocly/cli:2.76.0" "$(cat "$REPO/scripts/api-lint.sh")" "the image is pinned to a version"
done_testing
