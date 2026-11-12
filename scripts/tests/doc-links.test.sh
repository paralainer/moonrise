#!/usr/bin/env bash
# The repository's Markdown links resolve: every relative link in a tracked .md file (outside
# server/third_party) points at a file or folder that exists, and a #fragment into a Markdown file
# names one of its headings (GitHub's anchors). Links to web pages and mail addresses aren't
# fetched. Also checks that a broken link is reported, on a scratch file.
. "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

check_links() {   # check_links <root> <file>...: prints one line per broken link, exits 1 if any
  python3 -I - "$@" <<'PY'
import os, re, sys
root, files = sys.argv[1], sys.argv[2:]
link = re.compile(r'(?<!!)\[[^\]]*\]\(\s*<?([^)\s>]+)>?(?:\s+"[^"]*")?\s*\)|!\[[^\]]*\]\(\s*<?([^)\s>]+)>?')
ref = re.compile(r'^\s*\[[^\]]+\]:\s*(\S+)', re.M)
fence = re.compile(r'^(```|~~~).*?^\1', re.M | re.S)
code = re.compile(r'`[^`\n]*`')

def anchors(path):
    out, seen = set(), {}
    text = fence.sub('', open(path, encoding='utf-8').read())
    for m in re.finditer(r'^#{1,6}\s+(.*?)\s*#*\s*$', text, re.M):
        h = re.sub(r'\[([^\]]*)\]\([^)]*\)', r'\1', m.group(1))
        s = re.sub(r'[^\w\- ]', '', h.lower().replace('`', '')).replace(' ', '-')
        n = seen.get(s, 0)
        seen[s] = n + 1
        out.add(s if n == 0 else f'{s}-{n}')
    for m in re.finditer(r'<a\s+(?:name|id)="([^"]+)"', text):
        out.add(m.group(1))
    return out

bad = 0
for f in files:
    path = os.path.join(root, f)
    text = code.sub('', fence.sub('', open(path, encoding='utf-8').read()))
    targets = [a or b for a, b in link.findall(text)] + ref.findall(text)
    for t in targets:
        if re.match(r'^[a-z][a-z0-9+.-]*:', t, re.I):
            continue
        target, _, frag = t.partition('#')
        dest = os.path.normpath(os.path.join(os.path.dirname(path), target)) if target else path
        if not os.path.exists(dest):
            print(f'{f}: {t} (no such file)'); bad += 1; continue
        if frag and dest.endswith('.md') and frag not in anchors(dest):
            print(f'{f}: {t} (no such heading)'); bad += 1
sys.exit(1 if bad else 0)
PY
}

# ---- the checker finds a broken file link and a broken anchor, and passes good ones
mkdir -p "$SCRATCH/docs/sub"
printf '# Title\n\n## A `code` heading\n' > "$SCRATCH/docs/target.md"
cat > "$SCRATCH/docs/sub/page.md" <<'MD'
[ok](../target.md) [ok anchor](../target.md#a-code-heading) [web](https://example.org/x)
[missing](../nope.md) [bad anchor](../target.md#nothing)
`[not a link](in-code.md)`
MD
out=$(check_links "$SCRATCH" docs/sub/page.md); rc=$?
expect_status 1 "$rc" "a broken link fails" "$out"
expect_in "../nope.md (no such file)" "$out" "a missing file is reported"
expect_in "../target.md#nothing (no such heading)" "$out" "a missing heading is reported"
expect_not_in "a-code-heading" "$out" "a heading with code is found"
expect_not_in "in-code.md" "$out" "links inside code are ignored"
expect_not_in "example.org" "$out" "web links aren't checked"

# ---- the repository's own docs
mapfile -t docs < <(git -C "$REPO" ls-files '*.md' | grep -v '^server/third_party/')
out=$(check_links "$REPO" "${docs[@]}"); rc=$?
expect_status 0 "$rc" "every relative link in the repository's Markdown resolves" "$out"

done_testing
