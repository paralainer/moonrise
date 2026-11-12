#!/usr/bin/env bash
# clients/windows/build.ps1 gets libmpv from the project's own release first (tag
# deps-libmpv-<upstream tag>), the upstream only after it, and checks a pinned SHA-256; the
# notices name the same build. Also checks that a broken pin is reported, on a scratch copy.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

# check_pin <build.ps1> <THIRD-PARTY-NOTICES.txt>: prints what is wrong, exits 1 if anything is.
check_pin() {
  local ps=$1 notices=$2 bad=0 tag file sha urls
  val() { sed -n "s/^\\\$$1 = '\\([^']*\\)'.*/\\1/p" "$ps" | head -1; }
  tag=$(val LibMpvTag); file=$(val LibMpvFile); sha=$(val LibMpvSha256)
  [[ $tag =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}-[0-9a-f]+$ ]] || { echo "no \$LibMpvTag"; bad=1; }
  [[ $file == mpv-dev-lgpl-x86_64-*.7z ]] || { echo "\$LibMpvFile isn't an LGPL x86_64 build: '$file'"; bad=1; }
  [[ $sha =~ ^[0-9a-f]{64}$ ]] || { echo "\$LibMpvSha256 isn't a pinned SHA-256: '$sha'"; bad=1; }
  # The URL lines, in order, between "$LibMpvUrls = @(" and the closing ")".
  urls=$(sed -n '/^\$LibMpvUrls = @(/,/)$/p' "$ps" | grep -o '"https://[^"]*"' | tr -d '"')
  [[ $(sed -n 1p <<<"$urls") == 'https://github.com/$ProjectRepo/releases/download/deps-libmpv-$LibMpvTag/$LibMpvFile' ]] ||
    { echo "the first libmpv URL isn't the project's deps-libmpv release"; bad=1; }
  [[ $(sed -n 2p <<<"$urls") == 'https://github.com/zhongfly/mpv-winbuild/releases/download/$LibMpvTag/$LibMpvFile' ]] ||
    { echo "the upstream isn't the second (fallback) libmpv URL"; bad=1; }
  grep -q "^\$ProjectRepo = '[^/' ]*/[^/' ]*'" "$ps" || { echo "no \$ProjectRepo = '<owner>/<repo>'"; bad=1; }
  grep -qF -- '-ne $LibMpvSha256' "$ps" || { echo "nothing compares against \$LibMpvSha256"; bad=1; }
  [ -n "$tag" ] && ! grep -qF "build tag $tag" "$notices" && { echo "the notices don't name build tag $tag"; bad=1; }
  return $bad
}

PS="$REPO/clients/windows/build.ps1"
NOTICES="$REPO/clients/windows/THIRD-PARTY-NOTICES.txt"
out=$(check_pin "$PS" "$NOTICES"); rc=$?
expect_status 0 "$rc" "build.ps1 pins libmpv to the project's release" "$out"

sed "s/^\(\$LibMpvSha256 = \)'[^']*'/\1''/" "$PS" > "$SCRATCH/b1.ps1"
out=$(check_pin "$SCRATCH/b1.ps1" "$NOTICES"); rc=$?
expect_status 1 "$rc" "an empty hash is reported" "$out"
expect_in "isn't a pinned SHA-256" "$out" "empty hash message"

awk '/deps-libmpv/ { up = $0; next } { print } /zhongfly/ && up { print up; up = "" }' "$PS" > "$SCRATCH/b2.ps1"
out=$(check_pin "$SCRATCH/b2.ps1" "$NOTICES"); rc=$?
expect_status 1 "$rc" "the upstream first is reported" "$out"
expect_in "isn't the project's deps-libmpv release" "$out" "upstream-first message"

printf 'nothing\n' > "$SCRATCH/notices.txt"
out=$(check_pin "$PS" "$SCRATCH/notices.txt"); rc=$?
expect_status 1 "$rc" "notices naming another build are reported" "$out"
done_testing
