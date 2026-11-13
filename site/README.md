# The Moonrise landing page

A one-page site for Moonrise: plain HTML and CSS, no build step, nothing loaded from another host,
no cookies or analytics. It is dark by design (a screening room: true black, white and greys; the
films and the app bring the colour) and works at phone width. The only script is `motion.js`, which
plays the two muted videos (the film at the top at once, the app recording when it scrolls into
view) and runs their Pause buttons; without JavaScript, or with reduced motion, each shows its still
frame with the video's own controls.

| File | What it is |
|---|---|
| `index.html` | The page |
| `style.css` | Its styles |
| `fonts/` | Atkinson Hyperlegible Next, self-hosted (Latin subset, WOFF2), and its licence (`OFL.txt`) |
| `img/` | The icon at the sizes the page uses, made from `assets/brand/icon-glass-moonrise-play.png` |
| `film/` | The clip at the top: 28 seconds of *Tears of Steel* (WebM and MP4), its poster frame, and its English subtitles (`tos-en.vtt`) |
| `shots/` | Screenshots of the apps (WebP) and a real-time screen recording of the iPhone app (WebM and MP4, with a WebP still) |
| `motion.js` | Plays and pauses the two videos |
| `credits.html`, `CREDITS.md` | The sample films, posters, stills and the typeface: sources, authors, licences and changes, as a page the footer links to and as Markdown for the repository (the same content) |

## Preview

```sh
make site          # serves site/ on http://127.0.0.1:8000/ (Python 3); Ctrl-C stops it
```

Opening `site/index.html` straight from the disk shows the page, but browsers load subtitle tracks
(`film/tos-en.vtt`) only over HTTP, so the clip's subtitles need the local server.

`scripts/tests/site.test.sh` (part of `make check-quick`) checks that the page stays
self-contained: scripts only from its own `.js` files and with no network calls, nothing loaded from
another host, every file it names exists (the stylesheet's fonts and the subtitle track too), the
font has its licence beside it, every image has a size and a text alternative, and the folder stays
under 10 MB.

## Before publishing

- The links in `index.html` and `credits.html` point to `github.com/paralainer/moonrise`; a fork changes them to
  its own repository.
- The sample films, their art and the typeface are credited in the footer, `credits.html` and `CREDITS.md`; keep
  all three if they are replaced or moved.
- The icon is the Moonrise trademark ([TRADEMARKS.md](../TRADEMARKS.md)): a fork's site uses its
  own name and icon.

## Hosting

The page is published with GitHub Pages at <https://paralainer.github.io/moonrise/>. Pages
publishes from the repository root or `docs/`, not from `site/`, so a workflow does it:

1. `.github/workflows/pages.yml` runs on a push to `main` that touches `site/**`, or by hand
   (**Actions → Pages → Run workflow**). It runs `actions/configure-pages`,
   `actions/upload-pages-artifact` with `path: site`, and `actions/deploy-pages` (permissions
   `pages: write`, `id-token: write`; one deployment at a time), and nothing else: no build, no
   tests. `scripts/tests/pages-workflow.test.sh` checks that it uploads `site` and starts on nothing
   else, and that the other workflows stay manual only.
2. The repository's **Settings → Pages → Source** is **GitHub Actions** (a fork sets this once).
3. All paths in the page are relative, so it works under the `/moonrise/` sub-path unchanged.
4. A custom domain (optional): enter it under **Settings → Pages → Custom domain**, then point
   DNS at GitHub: a `CNAME` record to `paralainer.github.io` for a subdomain, or the `A`/`AAAA`
   records GitHub lists for an apex domain. Turn on **Enforce HTTPS** once the certificate is
   issued.

**Another static host** (Cloudflare Pages, Netlify, a web server you run): point it at the `site/`
folder with no build command, or copy the folder's contents to the web root.
