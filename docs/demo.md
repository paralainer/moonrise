# The demo ("Try the Demo")

The welcome screen offers **Try the Demo** next to *Connect to Your Server* (Apple TV: at the
bottom of the sign-in screen). It shows the app with a small library of open movies, with no
server: browsing, title pages, search, playback, audio, subtitles and watch progress work as with
a server. A yellow **DEMO** mark (Apple TV), a Demo note with **Exit Demo** on Home (iPhone, iPad,
Mac), "Demo Movies" rows and a credit line on every title page say what it is. **Settings → Demo
→ Exit Demo** goes back to the welcome screen; so does relaunching the app. Downloads, subtitle
search, quality changes, Manage Server and Tailscale are not part of the demo.

## The films

All by the Blender Foundation, under Creative Commons licences that allow this use with credit.
Metadata, posters (Wikimedia Commons), backdrops (frames of the films) and subtitles (the projects'
own) are bundled in the app (`clients/apple/Moonrise/Demo/Resources`, under 1 MB); the video streams from the
films' public hosts (no server of the Moonrise project):

| Film | Licence | Stream (first that answers) | Subtitles |
|---|---|---|---|
| Sintel (2010), dir. Colin Levy | CC BY 3.0, © Blender Foundation, durian.blender.org | archive.org `Sintel/sintel-2048-stereo.mp4` | French, Dutch, Russian |
| Tears of Steel (2012), dir. Ian Hubert | CC BY 3.0, © Blender Foundation, mango.blender.org | download.blender.org `ToS/tears_of_steel_720p.mov`, archive.org | English, German, Spanish, French, Russian |
| Big Buck Bunny (2008), dir. Sacha Goedegebure | CC BY 3.0, © Blender Foundation, peach.blender.org | test-streams.mux.dev HLS, archive.org | (no dialogue) |
| Elephants Dream (2006), dir. Bassam Kurdali | CC BY 2.5, Blender Foundation / Netherlands Media Art Institute, orange.blender.org | download.blender.org `ED/elephantsdream-1024-h264-st-aac.mov`, archive.org | (none published) |

Posters: Wikimedia Commons files "Big buck bunny poster big.jpg", "Sintel poster.jpg",
"Tos-poster.png" (CC BY 3.0, Blender Foundation) and "ElephantsDreamPoster.jpg" (CC BY 4.0,
Bassam Kurdali, Andy Goralczyk and the Blender Foundation).

The stream URLs were checked on 2026-11-05 (all four play on the iPhone and Apple TV simulators;
Google's old sample bucket and Bitmovin's Sintel HLS no longer answer). If a host goes away, edit
`DemoClient.films`.

## App Review notes (paste into App Store Connect → App Review Information → Notes)

> Moonrise is a player for the user's own media server (the free, open-source Moonrise server they run at
> home, e.g. on a NAS). Reviewers don't have one, so the app has a built-in demo: on the first
> screen tap **Try the Demo** (Apple TV: move down to **Try the Demo** at the bottom of the
> sign-in screen and press Select). It shows four Creative Commons films by the Blender
> Foundation (Big Buck Bunny, Elephants Dream, Sintel, Tears of Steel), streamed from their
> public hosts (archive.org, download.blender.org): browse Home and Library, open a title, play
> it, choose subtitles (Tears of Steel: English and more; Sintel: French, Dutch, Russian) and the
> audio track in the player. **Exit Demo** is on Home (iPhone/iPad/Mac) and in Settings → Demo.
> No account, sign-in or purchase exists in the app. The server software is at
> https://github.com/paralainer/moonrise (setup: run the Docker image, the app finds it on the local
> network and walks through a 6-digit setup code). The app uses the local network only to find
> and talk to the user's server (NSLocalNetworkUsageDescription).
