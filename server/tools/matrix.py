#!/usr/bin/env python3
"""Sample-title matrix for the Moonrise server: one title per format class, and checks against it.

  matrix.py --check info             GET /health and /v1/info
  matrix.py --check playback         GET /v1/items/{id}/playback for every sample; compare
                                     "supported" with the recorded expectation (exit 1 on a change)
  matrix.py --check playback --record   store the current results as the expectation
  matrix.py --check stream           for supported samples: master, playlists, init sections and
                                     segments (start and middle) of video (the variant served),
                                     the default audio and every converted audio track (whose two
                                     consecutive segments must also join without a gap or overlap)
  matrix.py --check transcode        for supported samples, each transcoded case (the lowest
                                     ladder rung; hdr=0 for HDR; burn-in of the first bitmap
                                     subtitle; the re-encoded original of codecs that can't be
                                     copied), opened mid-title: init and two segments; each
                                     segment's first fragment at its boundary, the second
                                     starting where the first ends, the same segment timeline as
                                     the copy, H.264/HEVC sample entry; DELETE closes the session.
                                     Compared with the recorded expectation (--record stores it)
  matrix.py --check subs             for samples with text subtitles: every text track from
                                     /v1/items/{id}/subtitles/{n}.srt (extracted by the server,
                                     or read from the sidecar): cues present,
                                     ending after they start; and the session's WebVTT rendition:
                                     X-TIMESTAMP-MAP=MPEGTS:0 and the SRT's first cue time
  matrix.py --check download         the original (/v1/items/{id}/download: ranges, ETag,
                                     If-Range) of one sample; prepared MP4 jobs (a remux of an
                                     MKV and a 1.5 Mb/s transcode): progress to done, then the
                                     file: ftyp+moov first (faststart), one video and the audio
                                     tracks asked for, the title's duration; DELETE removes it
  matrix.py --check all              info, playback, stream, transcode, subs, download
  matrix.py --check moonrise             end to end: /v1/me, browse, home, the samples (playback vs
                                     expectation, stream), text subtitles,
                                     a playback report cycle that leaves watch state as it was
                                     (start, progress and stop under 5 %), auth refusals; then
                                     the server's log since the start (over ssh) must show
                                     Moonrise-token requests and no connection to another media server

The samples (one title per format class, by Moonrise id, with paths and expectations) are your own
library's: $MOONRISE_MATRIX, else tools/local/matrix.json (git-ignored); see matrix.example.json for
the shape. Token: a Moonrise account token, from $MOONRISE_TOKEN, or the file named by --token-file /
$MOONRISE_TOKEN_FILE, sent as `Authorization: Bearer …` and never printed. Stream URLs carry
session ids only.
"""

import argparse
import json
import os
import re
import struct
import sys
import time
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
MATRIX = os.environ.get("MOONRISE_MATRIX") or os.path.join(HERE, "local", "matrix.json")
SERVER = os.environ.get("MOONRISE_URL", "http://localhost:8097")

def token(args=None):
    if os.environ.get("MOONRISE_TOKEN"):
        return os.environ["MOONRISE_TOKEN"].strip()
    path = (args and args.token_file) or os.environ.get("MOONRISE_TOKEN_FILE")
    if not path:
        sys.exit("no token: set MOONRISE_TOKEN or MOONRISE_TOKEN_FILE, or pass --token-file")
    return open(path).read().strip()


def headers(tok):
    return {"Authorization": "Bearer " + tok}


def get(url, hdrs=None, timeout=120, raw=False):
    req = urllib.request.Request(url, headers=hdrs or {})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        body = r.read()
        return (r.status, body) if raw else json.loads(body)


# ---------------------------------------------------------------- checks


def load():
    if not os.path.exists(MATRIX):
        sys.exit(f"no {MATRIX} (set MOONRISE_MATRIX; see tools/matrix.example.json)")
    return json.load(open(MATRIX))


def check_info(_args, _h):
    st, body = get(f"{SERVER}/health", raw=True)
    ok = st == 200 and body == b"ok"
    print(f"  /health   {st} {body[:20]!r} {'OK' if ok else 'FAIL'}")
    try:
        info = get(f"{SERVER}/v1/info")
        print(f"  /v1/info  {json.dumps(info)}")
        ok &= bool(info.get("server_id")) and isinstance(info.get("capabilities"), list)
    except urllib.error.HTTPError as e:
        print(f"  /v1/info  {e.code} (not served)")
        ok = False
    return ok


def item_id(sample):
    return sample["id"]


def playback(sample, h):
    t0 = time.time()
    try:
        iid = item_id(sample)
        info = get(f"{SERVER}/v1/items/{iid}/playback", h, timeout=180)
    except urllib.error.HTTPError as e:
        return {"error": f"HTTP {e.code}: {e.read()[:200].decode(errors='replace')}"}, time.time() - t0
    return info, time.time() - t0


def check_playback(args, h):
    m = load()
    ok = True
    for s in m["samples"]:
        info, dt = playback(s, h)
        if info.get("master"):
            # Opening it may have started a transcode (titles that can't be copied).
            delete(SERVER + info["master"].rsplit("/", 1)[0])
        got = {"supported": info.get("supported"), "reasons": info.get("reasons", [info.get("error")])}
        tracks = ""
        exp = s.get("expect")
        if args.record:
            s["expect"] = got
            status = "recorded"
        elif exp is None:
            status = "no expectation"
        elif exp["supported"] == got["supported"]:
            status = "same"
        else:
            status = f"CHANGED (was {exp['supported']})"
            ok = False if got["supported"] is not True else ok  # newly supported is progress
        print(f"  {s['class']:26} supported={str(got['supported']):5} {dt:5.1f}s  {status:16} "
              f"{'; '.join(r for r in got['reasons'] if r)[:90]} {tracks}")
    if args.record:
        json.dump(m, open(MATRIX, "w"), indent=1, ensure_ascii=False)
        print(f"recorded expectations in {MATRIX}")
    return ok


def is_mp4_piece(body, first):
    return len(body) > 8 and body[4:8] == first


def tfdt_span(seg, frame_ticks):
    """(decode time of a segment's first fragment, decode time after its last sample), for
    single-track fragments with per-sample durations in trun (converted audio)."""
    def boxes(pos, end):
        while pos + 8 <= end:
            size, kind = struct.unpack(">I4s", seg[pos:pos + 8])
            yield kind, pos + 8, pos + size
            pos += size
    start = end_t = None
    for kind, p, e in boxes(0, len(seg)):
        if kind != b"moof":
            continue
        for k2, p2, e2 in boxes(p, e):
            if k2 != b"traf":
                continue
            base, dur = None, 0
            for k3, p3, e3 in boxes(p2, e2):
                if k3 == b"tfdt":
                    base = struct.unpack(">Q", seg[p3 + 4:p3 + 12])[0] if seg[p3] == 1 else struct.unpack(">I", seg[p3 + 4:p3 + 8])[0]
                if k3 == b"trun":
                    dur = struct.unpack(">I", seg[p3 + 4:p3 + 8])[0] * frame_ticks
            start = base if start is None else start
            end_t = base + dur
    return start, end_t


def delete(url):
    req = urllib.request.Request(url, method="DELETE")
    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            return r.status
    except urllib.error.HTTPError as e:
        return e.code


def check_stream(_args, h):
    m = load()
    ok = True
    for s in m["samples"]:
        info, _ = playback(s, h)
        if not info.get("supported"):
            continue
        sid_base = info["master"].rsplit("/", 1)[0]  # /v1/s/{sid}
        base = SERVER + sid_base
        problems, t0 = [], time.time()
        st, master = get(base + "/master.m3u8", raw=True)
        if st != 200 or not master.startswith(b"#EXTM3U"):
            problems.append("master")
        # The variant served (older servers: the copy at video/).
        video = f"video/{info['variant']}" if info.get("variant") else "video"
        st, vpl = get(f"{base}/{video}/index.m3u8", raw=True)
        segs = [l for l in vpl.decode().splitlines() if l.endswith(".m4s")]
        picks = sorted({0, len(segs) // 2}) if segs else []
        pieces = [(f"{video}/init.mp4", b"ftyp")] + [(f"{video}/{n}.m4s", b"moof") for n in picks]
        audio = [a for a in info.get("audio", []) if a.get("default")] or info.get("audio", [])[:1]
        tracks = [t["index"] for t in audio[:1]] + [t["index"] for t in info.get("audio", []) if t.get("converted") and t not in audio[:1]]
        for a in tracks:
            get(base + f"/audio/{a}/index.m3u8", raw=True)
            pieces += [(f"audio/{a}/init.mp4", b"ftyp")] + [(f"audio/{a}/{n}.m4s", b"moof") for n in picks]
        for t in [t for t in info.get("audio", []) if t.get("converted")]:
            # Two consecutive segments of a converted track: the second starts where the first ends.
            a, n = t["index"], (picks[-1] if picks else 0)
            if n + 1 >= len(segs):
                continue
            try:
                frame = {"mp4a.40.2": 1024, "ec-3": 1536}[t["codecs_attr"]]
                ticks = frame * 1  # timescale == sample rate for converted audio
                s0 = tfdt_span(get(f"{base}/audio/{a}/{n}.m4s", timeout=180, raw=True)[1], ticks)
                s1 = tfdt_span(get(f"{base}/audio/{a}/{n + 1}.m4s", timeout=180, raw=True)[1], ticks)
                if s0[1] != s1[0]:
                    problems.append(f"audio/{a}: segment {n} ends at {s0[1]}, {n + 1} starts at {s1[0]}")
            except (urllib.error.HTTPError, KeyError, TypeError) as e:
                problems.append(f"audio/{a} continuity: {e}")
        for path, first in pieces:
            try:
                st, body = get(f"{base}/{path}", timeout=180, raw=True)
                if st != 200 or not is_mp4_piece(body, first):
                    problems.append(f"{path}: {st} {body[4:8]!r}")
            except urllib.error.HTTPError as e:
                problems.append(f"{path}: HTTP {e.code}")
        delete(base)
        ok &= not problems
        print(f"  {s['class']:26} {len(segs):5} segs  {time.time() - t0:5.1f}s  "
              f"{'OK' if not problems else 'FAIL ' + ', '.join(problems)}")
    return ok


def boxes(d, pos, end):
    while pos + 8 <= end:
        size, kind = struct.unpack(">I4s", d[pos:pos + 8])
        if size < 8:
            return
        yield kind, pos + 8, pos + size
        pos += size


def find_box(d, path, pos=0, end=None):
    end = len(d) if end is None else end
    for kind, p, e in boxes(d, pos, end):
        if kind == path[0]:
            return (p, e) if len(path) == 1 else find_box(d, path[1:], p, e)
    return None


def timescale(init):
    p, _ = find_box(init, [b"moov", b"trak", b"mdia", b"mdhd"])
    return struct.unpack(">I", init[p + (20 if init[p] == 1 else 12):][:4])[0]


def video_frags(seg):
    """[(tfdt, summed sample durations)] of each fragment (default or per-sample durations)."""
    out = []
    for kind, p, e in boxes(seg, 0, len(seg)):
        if kind != b"moof":
            continue
        traf = find_box(seg, [b"traf"], p, e)
        base, dur, defdur = None, 0, 0
        for k, q, qe in boxes(seg, *traf):
            flags = int.from_bytes(seg[q + 1:q + 4], "big")
            if k == b"tfhd":
                x = q + 8 + (8 if flags & 1 else 0) + (4 if flags & 2 else 0)
                defdur = struct.unpack(">I", seg[x:x + 4])[0] if flags & 8 else 0
            elif k == b"tfdt":
                base = struct.unpack(">Q", seg[q + 4:q + 12])[0] if seg[q] == 1 else struct.unpack(">I", seg[q + 4:q + 8])[0]
            elif k == b"trun":
                n = struct.unpack(">I", seg[q + 4:q + 8])[0]
                x = q + 8 + (4 if flags & 1 else 0) + (4 if flags & 4 else 0)
                for _ in range(n):
                    if flags & 0x100:
                        dur += struct.unpack(">I", seg[x:x + 4])[0]
                        x += 4
                    else:
                        dur += defdur
                    x += 4 * bin(flags & 0xE00).count("1")
        out.append((base, dur))
    return out


def playlist_times(text):
    t, out = 0.0, []
    for line in text.splitlines():
        if line.startswith("#EXTINF:"):
            out.append(round(t, 3))
            t += float(line[8:].rstrip(","))
    return out


def transcode_cases(s, info):
    """(name, query) of the transcoded cases a sample has."""
    cases = []
    variants = info.get("variants") or []
    if variants and variants[0].get("transcode"):
        cases.append(("original", ""))  # can't be copied: the original is a transcode
    lowest = [v for v in variants if v.get("transcode")]
    if lowest:
        cases.append((f"q{lowest[-1]['bitrate']}", f"&quality={lowest[-1]['bitrate']}"))
    if (info.get("video") or {}).get("range", "SDR") != "SDR":
        cases.append(("hdr0", "&hdr=0"))
    bitmap = [t for t in info.get("subtitles", []) if t.get("text") is False]
    if bitmap:
        cases.append((f"burn{bitmap[0]['index']}", f"&burn={bitmap[0]['index']}"))
    return cases


def check_transcode(args, h):
    m = load()
    ok = True
    for s in m["samples"]:
        info, _ = playback(s, h)
        if info.get("master"):
            delete(SERVER + info["master"].rsplit("/", 1)[0])
        if not info.get("supported") or "variants" not in info:
            continue
        copy_times = None
        results = {}
        for name, query in transcode_cases(s, info):
            t0 = time.time()
            problems = []
            try:
                mid = info["duration"] / 2
                pi = get(f"{SERVER}/v1/items/{item_id(s)}/playback?start={mid:.0f}{query}", h, timeout=60)
                if not pi.get("supported"):
                    results[name] = "unsupported: " + "; ".join(pi.get("reasons", []))
                    print(f"  {s['class']:26} {name:10} UNSUPPORTED {'; '.join(pi.get('reasons', []))}")
                    continue
                base = SERVER + pi["master"].rsplit("/", 1)[0]
                var = pi["variant"]
                if not next(v for v in pi["variants"] if v["id"] == var).get("transcode"):
                    delete(base)
                    results[name] = "busy: " + "; ".join(pi.get("notes", []))
                    print(f"  {s['class']:26} {name:10} BUSY (served {var}) {'; '.join(pi.get('notes', []))}")
                    continue
                if copy_times is None and any(v["id"] == "copy" for v in pi["variants"]):
                    copy_times = playlist_times(get(f"{base}/video/copy/index.m3u8", raw=True)[1].decode())
                times = playlist_times(get(f"{base}/video/{var}/index.m3u8", raw=True)[1].decode())
                if copy_times is not None and times != copy_times:
                    problems.append("segment timeline differs from the copy's")
                n = max(i for i, t in enumerate(times) if t <= mid)
                st, init = get(f"{base}/video/{var}/init.mp4", timeout=120, raw=True)
                start_s = time.time() - t0
                entry = b"hvc1" if b"hvc1" in init else b"avc1" if b"avc1" in init else None
                if not entry or find_box(init, [b"moov", b"trak", b"edts"]):
                    problems.append(f"init: entry {entry}, edts {bool(find_box(init, [b'moov', b'trak', b'edts']))}")
                ts = timescale(init)
                spans = []
                for k in (n, n + 1):
                    if k >= len(times):
                        continue
                    seg = get(f"{base}/video/{var}/{k}.m4s", timeout=120, raw=True)[1]
                    fr = video_frags(seg)
                    if not fr:
                        problems.append(f"segment {k}: no fragments")
                        continue
                    first, end = fr[0][0] / ts, (fr[-1][0] + fr[-1][1]) / ts
                    spans.append((k, first, end))
                    # AVI's keyframes decode a frame after their (dts) index time.
                    if not -0.02 <= first - times[k] <= 0.05:
                        problems.append(f"segment {k} starts at {first:.3f}, boundary {times[k]:.3f}")
                if len(spans) == 2 and abs(spans[0][2] - spans[1][1]) > 1e-6:
                    problems.append(f"segment {n} ends at {spans[0][2]:.4f}, {n + 1} starts at {spans[1][1]:.4f}")
                if delete(base) != 204:
                    problems.append("DELETE")
                results[name] = "ok" if not problems else "; ".join(problems)
                print(f"  {s['class']:26} {name:10} {var:32} first segment {start_s:4.1f}s  "
                      f"{'OK' if not problems else 'FAIL ' + ', '.join(problems)}")
            except (urllib.error.HTTPError, urllib.error.URLError, ValueError, TypeError, KeyError) as e:
                results[name] = f"error: {e}"
                print(f"  {s['class']:26} {name:10} ERROR {e}")
        exp = s.get("expect_transcode")
        if args.record:
            s["expect_transcode"] = results
        elif exp is not None:
            for name, r in results.items():
                if exp.get(name) == "ok" and r != "ok":
                    ok = False
        else:
            ok &= all(r == "ok" or r.startswith("unsupported") for r in results.values())
    if args.record:
        json.dump(m, open(MATRIX, "w"), indent=1, ensure_ascii=False)
        print(f"recorded transcode expectations in {MATRIX}")
    return ok


# ---------------------------------------------------------------- subtitles


def srt_cues(text):
    """[(start, end)] of an SRT's cues, in seconds."""
    def ts(x):
        x = x.strip().replace(",", ".")
        h, m, sec = x.split(":") if x.count(":") == 2 else ("0", *x.split(":"))
        return int(h) * 3600 + int(m) * 60 + float(sec)
    out = []
    for block in text.replace("\r\n", "\n").replace("\r", "\n").split("\n\n"):
        for line in block.splitlines():
            if "-->" in line:
                a, b = line.split("-->")
                try:
                    out.append((ts(a), ts(b.split()[0])))
                except (ValueError, IndexError):
                    pass
                break
    return out


def check_subs(_args, h):
    m = load()
    ok = True
    seen = set()
    for s in m["samples"]:
        if s["id"] in seen or not any(not x.split(" ")[0].isupper() for x in s["subtitles"]):
            continue
        seen.add(s["id"])
        problems, notes, t0 = [], [], time.time()
        try:
            tracks = [t for t in get(f"{SERVER}/v1/items/{item_id(s)}/subtitles", h)["subtitles"] if t.get("text")]
        except urllib.error.HTTPError as e:
            print(f"  {s['class']:26} FAIL list: HTTP {e.code}")
            ok = False
            continue
        first = None
        for t in tracks:
            idx = t["index"]
            try:
                ours = get(f"{SERVER}{t['url']}", h, timeout=1200, raw=True)[1].decode()
            except urllib.error.HTTPError as e:
                problems.append(f"{idx}: HTTP {e.code}")
                continue
            if t.get("source") == "opensubtitles":
                notes.append(f"{idx}: {len(srt_cues(ours))} cues (fetched)")
                continue
            cues = srt_cues(ours)
            if first is None and cues:
                first = (idx, cues[0][0])
            if not cues:
                problems.append(f"{idx}: no cues")
                continue
            if any(c[1] < c[0] for c in cues):
                problems.append(f"{idx}: a cue ends before it starts")
            a = cues
            notes.append(f"{idx}:{t.get('source', '?')[:3]} {len(a)} cues")
        # The HLS rendition of the first track: mapped 1:1 to the file's time.
        if first:
            info, _ = playback(s, h)
            if info.get("master"):
                base = SERVER + info["master"].rsplit("/", 1)[0]
                try:
                    vtt = get(f"{base}/subs/{first[0]}/cues.vtt", timeout=600, raw=True)[1].decode()
                    if "X-TIMESTAMP-MAP=MPEGTS:0,LOCAL:00:00:00.000" not in vtt:
                        problems.append("vtt: no MPEGTS:0 map")
                    cue = next((l for l in vtt.splitlines() if "-->" in l), None)
                    vt = srt_cues(cue.replace(".", ",") if cue else "")
                    if not vt or abs(vt[0][0] - first[1]) > 0.001:
                        problems.append(f"vtt: first cue {vt[0][0] if vt else None} vs srt {first[1]}")
                except urllib.error.HTTPError as e:
                    problems.append(f"vtt: HTTP {e.code}")
                delete(base)
        ok &= not problems
        print(f"  {s['class']:26} {len(tracks):2} text  {time.time() - t0:6.1f}s  "
              f"{'OK' if not problems else 'FAIL ' + ', '.join(problems)}  {' '.join(notes)[:110]}")
    return ok


# ---------------------------------------------------------------- downloads


def request(url, hdrs=None, method="GET", data=None, timeout=60):
    req = urllib.request.Request(url, headers=hdrs or {}, method=method, data=data)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, dict(r.headers), r.read()
    except urllib.error.HTTPError as e:
        return e.code, dict(e.headers), e.read()


def mp4_tracks(head):
    """(top-level box order, [(handler, sample entry)], movie duration) of an MP4's start."""
    order = [k for k, _, _ in boxes(head, 0, len(head))]
    moov = find_box(head, [b"moov"])
    tracks, duration = [], None
    if moov:
        mvhd = find_box(head, [b"mvhd"], *moov)
        if mvhd:
            p = mvhd[0]
            if head[p] == 1:
                ts, dur = struct.unpack(">IQ", head[p + 20:p + 32])
            else:
                ts, dur = struct.unpack(">II", head[p + 12:p + 20])
            duration = dur / ts if ts else None
        for kind, p, e in boxes(head, *moov):
            if kind != b"trak":
                continue
            hdlr = find_box(head, [b"mdia", b"hdlr"], p, e)
            stsd = find_box(head, [b"mdia", b"minf", b"stbl", b"stsd"], p, e)
            handler = head[hdlr[0] + 8:hdlr[0] + 12].decode() if hdlr else "?"
            entry = head[stsd[0] + 12:stsd[0] + 16].decode(errors="replace") if stsd else "?"
            tracks.append((handler, entry))
    return order, tracks, duration


def fetch_head(url, hdrs=None):
    """The start of an MP4 up to the end of its moov (when the moov comes first)."""
    st, hd, body = request(url, {**(hdrs or {}), "Range": "bytes=0-65535"})
    if st != 206:
        return st, hd, body
    need, pos = len(body), 0
    while pos + 8 <= len(body):
        size, kind = struct.unpack(">I4s", body[pos:pos + 8])
        if kind == b"moov":
            need = pos + size
            break
        if kind == b"mdat" or size < 8:
            break
        pos += size
    if need > len(body):
        st, hd, body = request(url, {**(hdrs or {}), "Range": f"bytes=0-{need - 1}"}, timeout=120)
    return st, hd, body


def check_download(_args, h):
    m = load()
    ok = True
    by_class = {s["class"]: s for s in m["samples"]}
    # The original: ranges, ETag, If-Range.
    s = by_class["mkv-hevc-sdr"]
    url = f"{SERVER}/v1/items/{item_id(s)}/download"
    problems = []
    st, hd, body = request(url, {**h, "Range": "bytes=0-1023"})
    hd = {k.lower(): v for k, v in hd.items()}
    total = int(hd.get("content-range", "/0").rsplit("/", 1)[1] or 0)
    if st != 206 or len(body) != 1024 or total != s["size"] or body[:4] != b"\x1aE\xdf\xa3":
        problems.append(f"range: {st} {len(body)} bytes of {total}")
    etag = hd.get("etag")
    if not etag:
        problems.append("no ETag")
    st2, hd2, tail = request(url, {**h, "Range": f"bytes={total - 100}-", "If-Range": etag or ""})
    if st2 != 206 or len(tail) != 100:
        problems.append(f"tail: {st2} {len(tail)}")
    # A changed file (another ETag): the whole file, so only read the status line.
    req = urllib.request.Request(url, headers={**h, "Range": "bytes=0-9", "If-Range": '"stale"'})
    with urllib.request.urlopen(req, timeout=60) as r:
        if r.status != 200:
            problems.append(f"stale If-Range: {r.status}")
    st3, _, _ = request(url)
    if st3 != 401:
        problems.append(f"no token: {st3}")
    ok &= not problems
    print(f"  original {s['class']:17} {total >> 20:6} MB  {'OK' if not problems else 'FAIL ' + ', '.join(problems)}")

    # Prepared MP4s: a remux of an MKV with every audio track (MP3 converted, E-AC-3 copied),
    # and a transcode.
    cases = [("remux", by_class["audio-mp3"], {"container": "mp4", "audio": "all"}),
             ("q1500000", by_class["mkv-h264"], {"container": "mp4", "quality": 1500000})]
    for name, s, params in cases:
        problems, t0 = [], time.time()
        if params.get("audio") == "all":
            info, _ = playback(s, h)
            if info.get("master"):
                delete(SERVER + info["master"].rsplit("/", 1)[0])
            params = {**params, "audio": [a["index"] for a in info.get("audio", [])]}
        st, _, body = request(f"{SERVER}/v1/items/{item_id(s)}/download-jobs", {**h, "Content-Type": "application/json"}, "POST",
                              json.dumps(params).encode())
        if st not in (200, 202):
            print(f"  job {name:12} {s['class']:22} FAIL create: {st} {body[:120]!r}")
            ok = False
            continue
        job = json.loads(body)
        while job["state"] in ("queued", "running") and time.time() - t0 < 1800:
            time.sleep(3)
            job = get(f"{SERVER}/v1/download-jobs/{job['id']}", h)
        mb = job.get("size", 0) >> 20
        if job["state"] != "done":
            problems.append(f"{job['state']}: {job.get('error')}")
        else:
            furl = SERVER + job["file"]
            st, hd, head = fetch_head(furl)
            order, tracks, duration = mp4_tracks(head)
            if order[:2] != [b"ftyp", b"moov"]:
                problems.append(f"boxes {order[:3]}")
            vids = [t for t in tracks if t[0] == "vide"]
            auds = [t for t in tracks if t[0] == "soun"]
            if len(vids) != 1 or vids[0][1] not in ("avc1", "hvc1"):
                problems.append(f"video {vids}")
            if len(auds) != len(job["audio"]) or any(a[1] not in ("mp4a", "ac-3", "ec-3") for a in auds):
                problems.append(f"audio {auds} for {job['audio']}")
            if duration is None or abs(duration - s["runtime"]) > 2:
                problems.append(f"duration {duration} vs {s['runtime']}")
            size = int(dict((k.lower(), v) for k, v in hd.items()).get("content-range", "/0").rsplit("/", 1)[1] or 0)
            if size != job["size"]:
                problems.append(f"size {size} vs {job['size']}")
            if request(SERVER + f"/v1/download-jobs/{job['id']}", h, "DELETE")[0] != 204:
                problems.append("delete")
            elif request(furl, {"Range": "bytes=0-9"})[0] != 404:
                problems.append("file still served after delete")
        ok &= not problems
        print(f"  job {name:12} {s['class']:22} {job.get('method', '?'):9} {mb:5} MB  {time.time() - t0:6.1f}s  "
              f"{'OK' if not problems else 'FAIL ' + ', '.join(problems)}")
    return ok


# ssh target of the server's Docker host, for the log check (unset: `docker logs` here).
MOONRISE_HOST = os.environ.get("MOONRISE_SSH")


def check_moonrise(args):
    import subprocess
    t_start = time.time() - 2
    h = headers(token(args))
    ok = True

    def step(name, fn):
        nonlocal ok
        t0 = time.time()
        try:
            detail = fn()
            print(f"  {name:34} {(time.time() - t0) * 1000:7.0f} ms  OK {detail or ''}")
        except Exception as e:  # noqa: BLE001 (report and go on)
            ok = False
            print(f"  {name:34} {(time.time() - t0) * 1000:7.0f} ms  FAIL {e}")

    def must(cond, msg):
        if not cond:
            raise AssertionError(msg)

    me = {}

    def s_me():
        me.update(get(f"{SERVER}/v1/me", h))
        must(me.get("id") and "name" in me, "no user")
        return f"admin={me.get('is_admin')}"
    step("GET /v1/me", s_me)
    libs = []

    def s_libs():
        libs.extend(get(f"{SERVER}/v1/libraries", h)["items"])
        must(libs, "no libraries")
        return ", ".join(f"{l['name']}={l['item_count']}" for l in libs)
    step("GET /v1/libraries", s_libs)

    def s_items():
        out = []
        for l in libs:
            r = get(f"{SERVER}/v1/items?library={l['id']}&limit=5", h)
            must(r["items"] and all(isinstance(i.get("user"), dict) for i in r["items"]), "items without user data")
            u = get(f"{SERVER}/v1/items?library={l['id']}&unplayed=1&limit=1", h)
            p = get(f"{SERVER}/v1/items?library={l['id']}&sort=played&limit=3", h)
            must("notes" not in u and "notes" not in p, f"filter/sort not applied: {u.get('notes') or p.get('notes')}")
            out.append(f"{l['name']}: {r['total']} total, {u['total']} unplayed")
            if l["kind"] == "shows":
                must("unplayed_item_count" in r["items"][0]["user"], "no unplayed_item_count")
        return "; ".join(out)
    step("GET /v1/items (unplayed, played sort)", s_items)

    def s_home():
        r = get(f"{SERVER}/v1/home", h)
        must(isinstance(r["resume"], list) and isinstance(r["next_up"], list) and "notes" not in r, "home rows missing")
        return f"resume={len(r['resume'])} next_up={len(r['next_up'])} latest=" + ",".join(f"{k}:{len(v)}" for k, v in r["latest"].items())
    step("GET /v1/home", s_home)
    step("GET /v1/search?q=the", lambda: f"{len(get(f'{SERVER}/v1/search?q=the', h)['titles'])} titles")

    def s_playback():
        bad = []
        for smp in load()["samples"]:
            info, _ = playback(smp, h)
            # None: an error (e.g. a DVD folder).
            if info.get("supported") != smp["expect"]["supported"]:
                bad.append(f"{smp['class']}: supported={info.get('supported')} {info.get('error') or info.get('reasons')}")
            if info.get("master"):
                delete(SERVER + info["master"].rsplit("/", 1)[0])
        must(not bad, "; ".join(bad))
        return f"{len(load()['samples'])} samples as expected"
    step("playback (samples)", s_playback)
    print("  stream (Moonrise token):")
    ok &= check_stream(None, h)

    def s_subs():
        n = 0
        for smp in load()["samples"]:
            if not smp["expect"]["supported"]:
                continue
            tracks = get(f"{SERVER}/v1/items/{item_id(smp)}/subtitles", h)["subtitles"]
            for t in [t for t in tracks if t.get("text")][:2]:
                st, body = get(f"{SERVER}/v1/items/{item_id(smp)}/subtitles/{t['index']}.vtt", h, timeout=300, raw=True)
                must(st == 200 and body.startswith(b"WEBVTT") and b"-->" in body, f"{smp['class']} track {t['index']}")
                n += 1
        return f"{n} text tracks"
    step("text subtitles (Moonrise token)", s_subs)

    def s_report():
        smp = next(x for x in load()["samples"] if x["expect"]["supported"])
        iid = item_id(smp)
        before = get(f"{SERVER}/v1/items/{iid}", h)["user"]
        info, _ = playback(smp, h)
        sid = info["session"]
        hj = {**h, "Content-Type": "application/json"}
        results = []
        for ev, pos in (("start", 0), ("progress", 10), ("stop", 10)):
            st, _, body = request(f"{SERVER}/v1/sessions/{sid}/{ev}", hj, "POST", json.dumps({"position": pos, "paused": False}).encode())
            must(st == 200, f"{ev}: {st} {body[:100]!r}")
            results.append(json.loads(body))
        must(all(r["recorded"] for r in results), "not recorded")
        must(results[-1]["outcome"] == "cleared" and results[-1]["user"]["position"] == 0, f"stop: {results[-1]}")
        after = get(f"{SERVER}/v1/items/{iid}", h)["user"]
        must(after["played"] == before["played"] and after["play_count"] == before["play_count"] and after["position"] == 0,
             f"watch state changed: {before} -> {after}")
        delete(SERVER + info["master"].rsplit("/", 1)[0])
        return f"{smp['class']}: recorded, position cleared, played={after['played']}"
    step("reports start/progress/stop", s_report)

    def s_refusals():
        st1, _, _ = request(f"{SERVER}/v1/home")
        st2, _, _ = request(f"{SERVER}/v1/home", {"Authorization": "Bearer moonrise_" + "A" * 43})
        st3, _, _ = request(f"{SERVER}/v1/sessions/nope/progress", {"Authorization": "Bearer moonrise_" + "A" * 43}, "POST", b"{}")
        must((st1, st2, st3) == (401, 401, 401), f"{st1} {st2} {st3}")
        return "401 without/with a bad token"
    step("auth refusals", s_refusals)

    def s_logs():
        since = time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime(t_start)) + "Z"
        cmd = f"docker logs --since {since} moonrise-server 2>&1"
        argv = ["ssh", MOONRISE_HOST, cmd] if MOONRISE_HOST else ["sh", "-c", cmd]
        out = subprocess.run(argv, capture_output=True, text=True, timeout=60).stdout
        out = re.sub(r"\x1b\[[0-9;]*m", "", out)
        moonrise_reqs = sum(1 for l in out.splitlines() if 'auth="moonrise"' in l or "auth=moonrise" in l)
        must(moonrise_reqs > 0, "no auth=moonrise lines in the log (is this the right server?)")
        other = [l for l in out.splitlines() if ":8096" in l or "jellyfin" in l.lower().replace("jellyfin-ffmpeg", "")]
        must(not other, f"{len(other)} log lines mention another media server: {other[:2]}")
        return f"{moonrise_reqs} log lines from Moonrise-token requests, none about another media server"
    step("server log: Moonrise only", s_logs)
    return ok


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--check", choices=["info", "playback", "stream", "transcode", "subs", "download", "all", "moonrise"])
    ap.add_argument("--record", action="store_true", help="with --check playback/transcode: store results as expectations")
    ap.add_argument("--token-file")
    a = ap.parse_args()
    if a.check == "moonrise":
        print("moonrise:")
        ok = check_moonrise(a)
        print("PASS" if ok else "FAIL")
        sys.exit(0 if ok else 1)
    if a.check:
        h = headers(token(a))
        checks = {"info": check_info, "playback": check_playback, "stream": check_stream, "transcode": check_transcode,
                  "subs": check_subs, "download": check_download}
        names = list(checks) if a.check == "all" else [a.check]
        ok = True
        for n in names:
            print(f"{n}:")
            ok &= checks[n](a, h)
        print("PASS" if ok else "FAIL")
        sys.exit(0 if ok else 1)
    if not a.check:
        ap.print_help()


if __name__ == "__main__":
    main()
