#!/usr/bin/env bash
# The build Mac sometimes lets an SSH handshake time out (banner exchange), and a retry gets
# through. scripts/common.sh tries the harmless contacts again (MAC_SSH_TRIES, default 3): the
# reachability check (`ssh true`) and rsync_retry. A long remote command (the build, the tests) is
# never retried, even when ssh's error looks like a connection that never opened: it may have
# started. ssh and rsync are stubs; nothing leaves here.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
C=$(fresh_clone)
mkdir -p "$SCRATCH/bin"

# ssh stub: the first $FAIL_FIRST calls fail as $FAIL_MSG (exit 255); then it runs the command here.
# Every call is counted in ssh-calls.
cat > "$SCRATCH/bin/ssh" <<STUB
#!/bin/bash
echo "ssh \$*" >> "$SCRATCH/ssh-calls"
n=\$(wc -l < "$SCRATCH/ssh-calls")
if [ "\$n" -le "\${FAIL_FIRST:-0}" ]; then
  echo "\${FAIL_MSG:-Connection timed out during banner exchange}" >&2
  echo "\${FAIL_MSG2:-Connection to UNKNOWN port 65535 timed out}" >&2
  exit 255
fi
while [ \$# -gt 0 ]; do case "\$1" in -o) shift 2 ;; -*) shift ;; *) break ;; esac; done
shift
[ \$# -gt 0 ] || exit 0
exec bash -c "\$*"
STUB
# rsync stub: exits with the codes in $RSYNC_CODES (space-separated) one call at a time, then 0.
cat > "$SCRATCH/bin/rsync" <<STUB
#!/bin/bash
echo "rsync \$*" >> "$SCRATCH/rsync-calls"
n=\$(wc -l < "$SCRATCH/rsync-calls")
code=\$(echo \${RSYNC_CODES:-} | cut -d' ' -f\$n)
exit \${code:-0}
STUB
chmod +x "$SCRATCH/bin/"*
run() { rm -f "$SCRATCH/ssh-calls" "$SCRATCH/rsync-calls"
  env PATH="$SCRATCH/bin:$PATH" MAC=tester@mac.invalid MAC_SSH_BACKOFF=0 "$@"; }
calls() { cat "$SCRATCH/$1-calls" 2>/dev/null | wc -l | tr -d ' '; }
lib() { run bash -c '. "$1/scripts/common.sh" 2>/dev/null; shift; eval "$*"' _ "$C" "$@"; }

out=$(FAIL_FIRST=2 lib 'need_mac_reachable && echo reachable' 2>&1); rc=$?
expect_status 0 $rc "two handshake timeouts, then through" "$out"
expect_in reachable "$out" "the third attempt reaches the Mac"
[ "$(calls ssh)" = 3 ] && ok || fail "two timeouts: $(calls ssh) ssh calls, wanted 3"
expect_in "trying again" "$out" "says it retries"
expect_in "ConnectTimeout=10" "$(cat "$SCRATCH/ssh-calls")" "ssh gets a ConnectTimeout"

out=$(FAIL_FIRST=9 lib 'need_mac_reachable; echo not-here' 2>&1); rc=$?
expect_status 1 $rc "the Mac never answers" "$out"
expect_in "can't reach the build Mac tester@mac.invalid over SSH, 3 attempts" "$out" "a clear message"
expect_in "ssh: Connection" "$out" "shows ssh's last error"
expect_not_in not-here "$out" "need_mac_reachable stops the script"
[ "$(calls ssh)" = 3 ] && ok || fail "never answers: $(calls ssh) ssh calls, wanted 3"

out=$(FAIL_FIRST=9 MAC_SSH_TRIES=5 lib 'mac_reachable || echo unreachable' 2>&1)
[ "$(calls ssh)" = 5 ] && ok || fail "MAC_SSH_TRIES=5: $(calls ssh) ssh calls"

out=$(FAIL_FIRST=9 FAIL_MSG='tester@mac.invalid: Permission denied (publickey).' FAIL_MSG2=x lib 'mac_reachable || echo unreachable' 2>&1)
expect_in unreachable "$out" "a refused key fails"
[ "$(calls ssh)" = 1 ] && ok || fail "a refused key: $(calls ssh) ssh calls, wanted 1 (no retry)"

for msg in 'kex_exchange_identification: read: Connection reset by peer' 'ssh: connect to host 192.0.2.1 port 22: Connection refused' 'Connection closed by 192.0.2.1 port 22'; do
  out=$(FAIL_FIRST=1 FAIL_MSG="$msg" FAIL_MSG2=x lib 'mac_reachable && echo reachable' 2>&1)
  expect_in reachable "$out" "retried after: $msg"
done

# need_mac_reachable passes under set -e after a retry.
out=$(FAIL_FIRST=1 lib 'set -e; need_mac_reachable; echo after' 2>&1)
expect_in after "$out" "need_mac_reachable under set -e"

# rsync_retry: a broken connection is retried, any other failure isn't.
out=$(RSYNC_CODES='255 12' lib 'rsync_retry -a src/ "$MAC:~/x/" && echo copied' 2>&1)
expect_in copied "$out" "rsync_retry: through on the third try"
[ "$(calls rsync)" = 3 ] && ok || fail "rsync_retry: $(calls rsync) rsync calls, wanted 3"
expect_in "-e ssh -o ConnectTimeout=10" "$(cat "$SCRATCH/rsync-calls")" "rsync's ssh gets a ConnectTimeout"
out=$(RSYNC_CODES='23' lib 'rsync_retry -a src/ "$MAC:~/x/"; echo rc=$?' 2>&1)
expect_in "rc=23" "$out" "rsync_retry: another failure is returned"
[ "$(calls rsync)" = 1 ] && ok || fail "rsync_retry: exit 23 retried ($(calls rsync) calls)"
out=$(RSYNC_CODES='255 255 255 255' lib 'rsync_retry -a src/ "$MAC:~/x/"; echo rc=$?' 2>&1)
expect_in "rc=255" "$out" "rsync_retry gives up"
[ "$(calls rsync)" = 3 ] && ok || fail "rsync_retry gives up after 3: $(calls rsync) calls"

# test-apple.sh: exit 3 (skipped) only after every attempt.
out=$(FAIL_FIRST=9 run bash "$C/scripts/test-apple.sh" 2>&1); rc=$?
expect_status 3 $rc "test-apple.sh with the Mac never answering" "$out"
expect_in "3 attempts" "$out" "test-apple.sh says it tried"
[ "$(calls ssh)" = 3 ] && ok || fail "test-apple.sh: $(calls ssh) ssh calls, wanted 3"
# A connection dropped mid-run: the stub answers `true`, but for the long command it prints the
# start marker and some output, then the error OpenSSH gives when a session breaks (the same words
# as a connection that never opened) and exits 255. The build/tests must run exactly once.
cat > "$SCRATCH/bin/ssh" <<STUB
#!/bin/bash
echo "ssh" >> "$SCRATCH/ssh-calls"
[ "\${@: -1}" = true ] && exit 0
echo "long: \${*:1:\$#-1}" >> "$SCRATCH/long-calls"   # one line a run: the options and host
echo "STARTED-LONG-COMMAND"
echo "** BUILD going"
echo "\${DROP_MSG}" >&2
exit 255
STUB
chmod +x "$SCRATCH/bin/ssh"
for drop in 'client_loop: ssh_packet_write_poll: Connection reset by 192.0.2.1 port 22' 'Connection closed by 192.0.2.1 port 22'; do
  for script in "test-apple.sh" "build.sh sim"; do
    rm -f "$SCRATCH/long-calls"
    out=$(DROP_MSG="$drop" run bash -c 'bash "$1"/scripts/$2' _ "$C" "$script" 2>&1); rc=$?
    [ $rc -ne 0 ] && ok || fail "$script dropped mid-run ($drop): exit 0" "$out"
    expect_in STARTED-LONG-COMMAND "$out" "$script: the long command started"
    [ "$(calls long)" = 1 ] && ok || fail "$script dropped mid-run ($drop): the long command ran $(calls long) times, wanted 1" "$out"
    expect_not_in "trying again" "$out" "$script: no retry after a mid-run drop"
    expect_in "ConnectTimeout=10" "$(cat "$SCRATCH/long-calls")" "$script: the long ssh gets a ConnectTimeout"
  done
done
done_testing
