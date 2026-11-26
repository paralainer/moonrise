#!/usr/bin/env python3
"""A controllable network gate: a TCP proxy between an app and a Moonrise server that can cut,
stall, throttle, delay, reset and flap the connection on command, for network tests (a phone
simulator downloading or streaming while the "network" fails). Python 3.9+, standard library only.

    netgate.py serve --upstream HOST:PORT --listen [NAME=]HOST:PORT [--listen ...] [--control HOST:PORT]
    netgate.py ctl [--control HOST:PORT] COMMAND [CHANNEL] [ARGS]

Each --listen is a channel (default name "lan"): the same server reached two ways, say the LAN
address and the tailnet address, can be cut one at a time. A channel is one of these modes:

    open       pass traffic (the default)
    cut        refuse new connections and reset the ones open: the server is gone
    stall      accept connections but pass nothing, and freeze the open ones: a dead link that
               still looks up (timeouts, not errors)

and has these knobs (0 or "off" turns one off):

    rate=BYTES/s     shared by every connection of the channel, each direction (k, m suffixes: 2m = 2 MB/s)
    delay=MS         added to every chunk, each direction (latency)
    jitter=MS        up to this much more, at random
    loss=P           chance (0..1) per chunk of a stall of 200-1500 ms, as a lost packet's retransmit
    reset=P          chance (0..1) per chunk that the connection is reset there
    drop_after=BYTES each connection is reset after passing this much from the server

Control commands (also over HTTP: GET /state, POST /set?channel=lan&mode=cut&rate=2m, POST /kill, POST /flap):

    state                         the channels, their settings and counters (JSON)
    open|cut|stall [CHANNEL]      set the mode (no CHANNEL: every channel)
    set [CHANNEL] KEY=VALUE ...   set knobs (and mode=...); `set lan off` clears them all
    kill [CHANNEL]                reset the open connections once (mid-transfer drop); new ones pass
    flap [CHANNEL] UP DOWN COUNT  COUNT times: UP seconds open, DOWN seconds cut, then open
    log [N]                       the last N events (connections, HTTP request lines, resets)
    reset-counters

The request log sniffs HTTP/1.x request lines on the way to the server, so a test can see which
segments the app asked for and when (keep-alive connections included; TLS shows nothing).
"""
import argparse
import asyncio
import json
import random
import re
import sys
import time
import urllib.parse
import urllib.request

CHUNK = 64 * 1024
REQ_LINE = re.compile(rb"(GET|POST|PUT|DELETE|HEAD|PATCH|OPTIONS) (\S+) HTTP/1\.[01]\r\n")


def parse_size(v):
    """'2m' -> 2_000_000, '512k' -> 512_000, '0'/'off' -> 0."""
    v = str(v).strip().lower()
    if v in ("", "off", "none", "0"):
        return 0
    mult = 1
    if v[-1] in "kmg":
        mult = {"k": 1_000, "m": 1_000_000, "g": 1_000_000_000}[v[-1]]
        v = v[:-1]
    return int(float(v) * mult)


def parse_prob(v):
    v = str(v).strip().lower()
    if v in ("", "off", "none"):
        return 0.0
    p = float(v.rstrip("%")) / (100 if v.endswith("%") else 1)
    if not 0 <= p <= 1:
        raise ValueError(f"not a probability: {v}")
    return p


KNOBS = {"rate": parse_size, "delay": parse_size, "jitter": parse_size, "loss": parse_prob,
         "reset": parse_prob, "drop_after": parse_size}
MODES = ("open", "cut", "stall")


class Bucket:
    """A token bucket shared by a channel's connections (bytes per second)."""

    def __init__(self):
        self.tokens = 0.0
        self.at = time.monotonic()

    async def take(self, n, rate):
        if rate <= 0:
            return
        while True:
            now = time.monotonic()
            self.tokens = min(rate * 0.25, self.tokens + (now - self.at) * rate)
            self.at = now
            if self.tokens >= n or self.tokens >= rate * 0.25:
                self.tokens -= n
                return
            await asyncio.sleep(max(0.005, (min(n, rate * 0.25) - self.tokens) / rate))


class Channel:
    def __init__(self, name, host, port):
        self.name, self.host, self.port = name, host, port
        self.mode = "open"
        self.knobs = {k: 0 for k in KNOBS}
        self.conns = set()
        self.buckets = {"up": Bucket(), "down": Bucket()}
        self.flap_task = None
        self.counters = {}
        self.reset_counters()

    def reset_counters(self):
        self.counters = {"connections": 0, "refused": 0, "resets": 0, "bytes_up": 0, "bytes_down": 0, "requests": 0}

    def state(self):
        return {"listen": f"{self.host}:{self.port}", "mode": self.mode, **self.knobs,
                "open_connections": len(self.conns), "flapping": bool(self.flap_task and not self.flap_task.done()),
                **self.counters}


class Conn:
    def __init__(self, ch, cw, sr, sw):
        self.ch, self.cw, self.sr, self.sw = ch, cw, sr, sw
        self.closed = False
        self.down_bytes = 0
        self.tasks = []

    def abort(self):
        """Reset both sides (RST where the platform allows)."""
        if self.closed:
            return
        self.closed = True
        for w in (self.cw, self.sw):
            try:
                w.transport.abort()
            except Exception:
                pass
        for t in self.tasks:
            t.cancel()


class Gate:
    def __init__(self, upstream, log_size=2000):
        self.up_host, self.up_port = upstream
        self.channels = {}
        self.events = []
        self.log_size = log_size
        self.t0 = time.time()

    def event(self, ch, kind, text=""):
        e = {"t": round(time.time(), 3), "channel": ch.name if ch else "-", "event": kind, "text": text}
        self.events.append(e)
        del self.events[:-self.log_size]

    # -- proxying -------------------------------------------------------------------------------

    async def handle(self, ch, cr, cw):
        if ch.mode == "cut":
            ch.counters["refused"] += 1
            cw.transport.abort()
            return
        while ch.mode == "stall":
            # Accepted, but nothing passes until the channel opens (or it is cut).
            await asyncio.sleep(0.2)
        if ch.mode == "cut":
            ch.counters["refused"] += 1
            cw.transport.abort()
            return
        try:
            sr, sw = await asyncio.open_connection(self.up_host, self.up_port)
        except OSError as e:
            self.event(ch, "upstream-error", str(e))
            cw.transport.abort()
            return
        c = Conn(ch, cw, sr, sw)
        ch.conns.add(c)
        ch.counters["connections"] += 1
        peer = cw.get_extra_info("peername")
        self.event(ch, "connect", f"{peer[0]}:{peer[1]}" if peer else "")
        c.tasks = [asyncio.ensure_future(self.pump(c, cr, sw, "up")), asyncio.ensure_future(self.pump(c, sr, cw, "down"))]
        try:
            await asyncio.gather(*c.tasks, return_exceptions=True)
        finally:
            ch.conns.discard(c)
            if not c.closed:
                c.closed = True
                for w in (cw, sw):
                    try:
                        w.close()
                    except Exception:
                        pass

    async def feed(self, c, reader, queue, direction):
        """Read one direction into the queue, each chunk stamped with when it may be delivered
        (now + delay + jitter): latency delays every chunk without limiting the throughput."""
        ch = c.ch
        due = 0.0
        try:
            while not c.closed:
                data = await reader.read(CHUNK)
                if direction == "up" and data:
                    for m in REQ_LINE.finditer(data):
                        ch.counters["requests"] += 1
                        self.event(ch, "request", f"{m.group(1).decode()} {m.group(2).decode(errors='replace')}")
                k = ch.knobs
                # In order: never before the chunk ahead of it.
                due = max(due, time.monotonic() + (k["delay"] + random.random() * k["jitter"]) / 1000)
                await queue.put((due, data))
                if not data:
                    return
        except (ConnectionError, asyncio.IncompleteReadError, OSError):
            await queue.put((0, None))
        except asyncio.CancelledError:
            pass

    async def pump(self, c, reader, writer, direction):
        ch = c.ch
        queue = asyncio.Queue(maxsize=64)
        feeder = asyncio.ensure_future(self.feed(c, reader, queue, direction))
        c.tasks.append(feeder)
        try:
            while not c.closed:
                due, data = await queue.get()
                if data is None:
                    return c.abort()
                wait = due - time.monotonic()
                if wait > 0:
                    await asyncio.sleep(wait)
                if not data:
                    # Half-close: tell the other side, keep the other direction going.
                    try:
                        writer.write_eof()
                    except Exception:
                        pass
                    return
                while ch.mode == "stall" and not c.closed:
                    await asyncio.sleep(0.1)
                if ch.mode == "cut" or c.closed:
                    return c.abort()
                k = ch.knobs
                if k["loss"] and random.random() < k["loss"]:
                    await asyncio.sleep(random.uniform(0.2, 1.5))
                if k["reset"] and random.random() < k["reset"]:
                    ch.counters["resets"] += 1
                    self.event(ch, "reset", f"random, {direction}")
                    return c.abort()
                await ch.buckets[direction].take(len(data), k["rate"])
                if direction == "down" and k["drop_after"] and c.down_bytes + len(data) > k["drop_after"]:
                    cut_at = max(0, k["drop_after"] - c.down_bytes)
                    if cut_at:
                        writer.write(data[:cut_at])
                        await writer.drain()
                    ch.counters["resets"] += 1
                    self.event(ch, "reset", f"drop_after {k['drop_after']} bytes")
                    return c.abort()
                writer.write(data)
                await writer.drain()
                if direction == "down":
                    c.down_bytes += len(data)
                ch.counters["bytes_" + direction] += len(data)
        except (ConnectionError, asyncio.IncompleteReadError, OSError):
            c.abort()
        except asyncio.CancelledError:
            pass
        finally:
            feeder.cancel()

    # -- control --------------------------------------------------------------------------------

    def pick(self, name):
        if not name or name == "all":
            return list(self.channels.values())
        if name not in self.channels:
            raise ValueError(f"no channel {name!r} (have: {', '.join(self.channels)})")
        return [self.channels[name]]

    def set_mode(self, chs, mode):
        if mode not in MODES:
            raise ValueError(f"mode must be one of {MODES}")
        for ch in chs:
            if ch.flap_task and not ch.flap_task.done() and mode != "_flap":
                ch.flap_task.cancel()
            ch.mode = mode
            self.event(ch, "mode", mode)
            if mode == "cut":
                self.kill(ch, why="cut")

    def kill(self, ch, why="kill"):
        n = len(ch.conns)
        for c in list(ch.conns):
            c.abort()
        if n:
            ch.counters["resets"] += n
        self.event(ch, "kill", f"{why}: {n} connections reset")
        return n

    async def flap(self, ch, up, down, count):
        try:
            for _ in range(count):
                ch.mode = "cut"
                self.kill(ch, why="flap")
                self.event(ch, "mode", "cut (flap)")
                await asyncio.sleep(down)
                ch.mode = "open"
                self.event(ch, "mode", "open (flap)")
                await asyncio.sleep(up)
        except asyncio.CancelledError:
            pass

    def apply(self, path, params):
        chs = self.pick(params.get("channel"))
        if path == "/state":
            return {"upstream": f"{self.up_host}:{self.up_port}", "channels": {c.name: c.state() for c in self.channels.values()}}
        if path == "/set":
            for key, val in params.items():
                if key == "channel":
                    continue
                if key == "mode":
                    self.set_mode(chs, val)
                elif key == "off":
                    for ch in chs:
                        ch.knobs = {k: 0 for k in KNOBS}
                        self.event(ch, "knobs", "off")
                elif key in KNOBS:
                    v = KNOBS[key](val)
                    for ch in chs:
                        ch.knobs[key] = v
                        self.event(ch, "knob", f"{key}={v}")
                else:
                    raise ValueError(f"unknown setting {key!r}")
            return {c.name: c.state() for c in chs}
        if path == "/kill":
            return {c.name: self.kill(c) for c in chs}
        if path == "/flap":
            up, down, count = float(params.get("up", 5)), float(params.get("down", 5)), int(params.get("count", 3))
            for ch in chs:
                if ch.flap_task and not ch.flap_task.done():
                    ch.flap_task.cancel()
                ch.flap_task = asyncio.ensure_future(self.flap(ch, up, down, count))
            return {"flapping": [c.name for c in chs], "up": up, "down": down, "count": count}
        if path == "/log":
            n = int(params.get("n", 50))
            return self.events[-n:]
        if path == "/reset-counters":
            for ch in chs:
                ch.reset_counters()
            return {"ok": True}
        raise ValueError(f"unknown path {path}")

    async def control(self, reader, writer):
        try:
            head = await reader.readuntil(b"\r\n\r\n")
            line = head.split(b"\r\n", 1)[0].decode()
            _method, target, _ = line.split(" ", 2)
            u = urllib.parse.urlsplit(target)
            params = dict(urllib.parse.parse_qsl(u.query, keep_blank_values=True))
            try:
                body, status = json.dumps(self.apply(u.path, params), indent=1), "200 OK"
            except ValueError as e:
                body, status = json.dumps({"error": str(e)}), "400 Bad Request"
            data = body.encode()
            writer.write(f"HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {len(data)}\r\nConnection: close\r\n\r\n".encode() + data)
            await writer.drain()
        except Exception:
            pass
        finally:
            writer.close()


def hostport(s, default_host="0.0.0.0"):
    host, _, port = s.rpartition(":")
    return (host or default_host).strip("[]"), int(port)


async def serve(args):
    gate = Gate(hostport(args.upstream, "127.0.0.1"))
    servers = []
    for spec in args.listen:
        name, _, addr = spec.rpartition("=")
        name = name or ("lan" if not gate.channels else f"ch{len(gate.channels)}")
        host, port = hostport(addr)
        ch = Channel(name, host, port)
        gate.channels[name] = ch
        servers.append(await asyncio.start_server(lambda r, w, ch=ch: gate.handle(ch, r, w), host, port, reuse_address=True))
        print(f"netgate: channel {name} on {host}:{port} -> {args.upstream}", flush=True)
    ch_host, ch_port = hostport(args.control, "127.0.0.1")
    servers.append(await asyncio.start_server(gate.control, ch_host, ch_port, reuse_address=True))
    print(f"netgate: control on {ch_host}:{ch_port}", flush=True)
    await asyncio.gather(*(s.serve_forever() for s in servers))


def ctl(args):
    words = list(args.words)
    if not words:
        sys.exit("usage: netgate.py ctl COMMAND [CHANNEL] [ARGS]")
    cmd, rest = words[0], words[1:]
    params = {}

    def channel_arg():
        if rest and "=" not in rest[0] and not rest[0].replace(".", "").isdigit() and rest[0] != "off":
            params["channel"] = rest.pop(0)

    if cmd in MODES:
        channel_arg()
        path, params["mode"] = "/set", cmd
    elif cmd == "set":
        channel_arg()
        path = "/set"
        for kv in rest:
            if kv == "off":
                params["off"] = "1"
                continue
            k, _, v = kv.partition("=")
            params[k] = v
    elif cmd == "kill":
        channel_arg()
        path = "/kill"
    elif cmd == "flap":
        channel_arg()
        path = "/flap"
        for k, v in zip(("up", "down", "count"), rest):
            params[k] = v
    elif cmd == "log":
        path = "/log"
        if rest:
            params["n"] = rest[0]
    elif cmd in ("state", "reset-counters"):
        channel_arg()
        path = "/" + cmd
    else:
        sys.exit(f"unknown command {cmd!r}")
    url = f"http://{args.control}{path}?{urllib.parse.urlencode(params)}"
    req = urllib.request.Request(url, method="GET" if path in ("/state", "/log") else "POST")
    try:
        with urllib.request.urlopen(req, timeout=10) as r:
            print(r.read().decode())
    except urllib.error.HTTPError as e:
        sys.exit(f"netgate: {e.read().decode()}")
    except OSError as e:
        sys.exit(f"netgate: control at {args.control} not reachable: {e}")


def main():
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = p.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("serve")
    s.add_argument("--upstream", required=True, help="the server, HOST:PORT")
    s.add_argument("--listen", action="append", required=True, help="[NAME=]HOST:PORT, one per channel")
    s.add_argument("--control", default="127.0.0.1:8790")
    c = sub.add_parser("ctl")
    c.add_argument("--control", default="127.0.0.1:8790")
    c.add_argument("words", nargs=argparse.REMAINDER)
    args = p.parse_args()
    if args.cmd == "serve":
        try:
            asyncio.run(serve(args))
        except KeyboardInterrupt:
            pass
    else:
        ctl(args)


if __name__ == "__main__":
    main()
