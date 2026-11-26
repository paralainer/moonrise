#!/usr/bin/env bash
# scripts/netgate.py, the network gate the app network tests put between a simulator and a server:
# traffic passes when open; cut refuses new connections and resets open ones; stall passes nothing;
# rate throttles; drop_after resets a transfer part-way; kill resets the open connections once;
# flap cuts and opens on a timer; the request log sees the HTTP request lines. Everything runs on
# 127.0.0.1 against a small local HTTP server; nothing leaves the machine.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
command -v python3 >/dev/null || { echo "netgate.test.sh: skipped (no python3)"; exit 0; }
GATE="$REPO/scripts/netgate.py"

free_port() { python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])'; }
UP=$(free_port); LAN=$(free_port); TS=$(free_port); CTL=$(free_port)

# The "server": 4 MB at /big, a few bytes at /small.
mkdir -p "$SCRATCH/www"
head -c 4000000 /dev/zero > "$SCRATCH/www/big"
echo hello > "$SCRATCH/www/small"
python3 -m http.server "$UP" --bind 127.0.0.1 --directory "$SCRATCH/www" >/dev/null 2>&1 &
HTTP_PID=$!
python3 "$GATE" serve --upstream "127.0.0.1:$UP" --listen "lan=127.0.0.1:$LAN" --listen "tailnet=127.0.0.1:$TS" \
  --control "127.0.0.1:$CTL" > "$SCRATCH/gate.log" 2>&1 &
GATE_PID=$!
trap 'kill $HTTP_PID $GATE_PID 2>/dev/null; rm -rf "$SCRATCH"' EXIT
ctl() { python3 "$GATE" ctl --control "127.0.0.1:$CTL" "$@"; }
for _ in $(seq 50); do ctl state >/dev/null 2>&1 && curl -s -o /dev/null "http://127.0.0.1:$UP/small" && break; sleep 0.1; done

# get <port> <path> [max seconds]: prints "<curl exit> <http code> <bytes> <seconds>".
get() {
  local out rc
  out=$(curl -s -o /dev/null -m "${3:-10}" -w '%{http_code} %{size_download} %{time_total}' "http://127.0.0.1:$1$2"); rc=$?
  echo "$rc $out"
}

# Open: both channels pass, whole files.
r=$(get "$LAN" /big); expect_in "0 200 4000000" "$r" "open lan passes the whole file"
r=$(get "$TS" /small); expect_in "0 200 6" "$r" "open tailnet passes"
st=$(ctl state); expect_in '"mode": "open"' "$st" "state shows the mode"
log=$(ctl log 20); expect_in "GET /big" "$log" "the request log has the request line"

# Cut one channel: it refuses, the other still passes.
ctl cut lan >/dev/null
r=$(get "$LAN" /small 3); case "$r" in "0 "*) fail "cut lan still answers" "$r" ;; *) ok ;; esac
r=$(get "$TS" /small); expect_in "0 200 6" "$r" "cutting lan leaves tailnet open"
ctl open lan >/dev/null
r=$(get "$LAN" /small); expect_in "0 200 6" "$r" "open again passes"

# Stall: the request hangs (curl times out, exit 28) instead of failing at once.
ctl stall lan >/dev/null
r=$(get "$LAN" /small 2); expect_in "28 " "$r" "stall hangs until the timeout"
ctl open lan >/dev/null

# Rate: 4 MB at 2 MB/s takes over 1.4 s.
ctl set lan rate=2m >/dev/null
r=$(get "$LAN" /big 20)
secs=$(awk '{print $4}' <<<"$r")
if awk -v s="$secs" 'BEGIN { exit !(s > 1.4) }'; then ok; else fail "rate=2m: 4 MB took $secs s" "$r"; fi
ctl set lan off >/dev/null
r=$(get "$LAN" /big); secs=$(awk '{print $4}' <<<"$r")
if awk -v s="$secs" 'BEGIN { exit !(s < 1.4) }'; then ok; else fail "set off: still slow ($secs s)" "$r"; fi

# delay: every chunk arrives 300 ms late, but the throughput stays (4 MB in well under 2 s, where
# 300 ms per 64 KB chunk in turn would take 19 s).
ctl set lan delay=300 >/dev/null
r=$(get "$LAN" /small); secs=$(awk '{print $4}' <<<"$r")
if awk -v s="$secs" 'BEGIN { exit !(s >= 0.3) }'; then ok; else fail "delay=300: answered in $secs s" "$r"; fi
r=$(get "$LAN" /big 20); secs=$(awk '{print $4}' <<<"$r")
if awk -v s="$secs" 'BEGIN { exit !(s < 2) }'; then ok; else fail "delay=300: 4 MB took $secs s (latency limits throughput)" "$r"; fi
expect_in "0 200 4000000" "$r" "delay passes the whole file"
ctl set lan off >/dev/null

# drop_after: the transfer is reset part-way (curl: 18 partial file or 56 receive error).
ctl set lan drop_after=1m >/dev/null
r=$(get "$LAN" /big)
case "$r" in 18\ *|56\ *) ok ;; *) fail "drop_after=1m didn't cut the transfer" "$r" ;; esac
bytes=$(awk '{print $3}' <<<"$r")
if [ "${bytes:-0}" -le 1000000 ]; then ok; else fail "drop_after=1m passed $bytes bytes"; fi
ctl set lan off >/dev/null

# kill: a slow transfer in progress is reset; the next request passes.
ctl set lan rate=500k >/dev/null
get "$LAN" /big 20 > "$SCRATCH/killed" &
sleep 1
ctl kill lan >/dev/null
wait $!
r=$(cat "$SCRATCH/killed"); case "$r" in "0 "*) fail "kill didn't reset the open transfer" "$r" ;; *) ok ;; esac
ctl set lan off >/dev/null
r=$(get "$LAN" /small); expect_in "0 200 6" "$r" "after kill, new connections pass"

# Flap: down for the first second, then up again.
ctl flap lan 2 1 1 >/dev/null
sleep 0.2
r=$(get "$LAN" /small 1); case "$r" in "0 "*) fail "flap: not cut while down" "$r" ;; *) ok ;; esac
sleep 1.3
r=$(get "$LAN" /small); expect_in "0 200 6" "$r" "flap: open again after the down time"

# Bad input is refused with a message.
out=$(ctl set lan loss=2 2>&1); expect_in "not a probability" "$out" "loss over 1 is refused"
out=$(ctl cut nowhere 2>&1); expect_in "no channel" "$out" "an unknown channel is refused"

done_testing
