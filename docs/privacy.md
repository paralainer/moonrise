# Privacy

Moonrise has **no analytics, no crash reporting, no accounts with us and no server of ours**. Your
library, accounts and watch history stay on your server. This is everything that leaves your
network, and when (checked against the code for this release).

## The server

| Destination | When | What is sent |
|---|---|---|
| TMDB (`api.themoviedb.org`, `image.tmdb.org`) | identifying titles and refreshing their details, unless no TMDB key is set | search terms made from file and folder names (title, year), TMDB ids, the API key (Moonrise's built-in one or yours); artwork downloads |
| OpenSubtitles (`api.opensubtitles.com`) | only when someone uses *Search Online* for subtitles, or an admin checks an account | the title's IMDb/TMDB id, season and episode, the language, the file's OpenSubtitles hash (computed from 128 KB of it), the API key, and your OpenSubtitles login if you added one |
| The LLM provider an admin chooses (Anthropic, OpenAI, a ChatGPT plan, Google Gemini, OpenRouter, or a model server of your own such as Ollama, LM Studio or llama.cpp, where nothing leaves your network) | only if an admin sets one up: for titles it can't match by name, and when an admin presses Test | the file's path relative to the library folder, what was read from it (title, year, season, episode, runtime, codecs and audio languages) and the TMDB candidates (titles, years, runtimes, overviews); the API key (only to the provider and address it was saved for), or for a ChatGPT plan the signed-in account's token (only to OpenAI) |
| OpenAI's sign-in (`auth.openai.com`) | only if an admin signs in with ChatGPT: the sign-in itself, then about once an hour to renew it, once a day for OpenAI's public signing keys, and at sign-out to end the sign-in | the sign-in tokens and the server's random installation id (no library data) |
| IMDb (`datasets.imdbws.com`) | about once a week, unless ratings are switched off | nothing but the download of IMDb's public ratings file |
| OMDb (`www.omdbapi.com`) | only with an OMDb key set on the server: films and shows, slowly in the background | the title's IMDb id and the key |
| TheIntroDB (`api.theintrodb.org`), IntroDB (`api.introdb.app`) | when a film or an episode is opened or played and its intro and credits times aren't kept yet (switch `community_segments`) | the title's (an episode's show's) TVDB, TMDB or IMDb id, season and episode numbers, the episode's TVDB id and the file's length |
| Your SMB shares | reading your videos | your share credentials, to your NAS only |
| Your local network | Bonjour (mDNS) announcement of the server's name and port | |

Nothing else: no update checks, no usage statistics. The server's log stays on the server (the
apps' log view shows it to admins, with keys, passwords and tokens redacted).

## The apps

The apps (Apple, Android and Windows) talk to your Moonrise server and to nothing else, apart from
the three exceptions below. Every third-party lookup in the table above (TMDB, OpenSubtitles, OMDb,
IMDb, TheIntroDB, IntroDB, the LLM) is the server's, and so are the keys
([ADR 0013](adr/0013-the-server-is-the-only-integration-point.md)): no app looks anything up there
itself, and artwork comes from your server too.

| Destination | When | What is sent |
|---|---|---|
| Your Moonrise server | always | your sign-in, browsing, playback and watch progress (progress watched offline is kept on the device and sent when the server is back); on Apple and Android, *Settings → Use Online Databases* off asks it to leave TheIntroDB's and IntroDB's markers out |
| OpenAI's sign-in page (`auth.openai.com`) | iPhone, iPad and Mac only: when an admin signs in with ChatGPT in Manage Server | an ordinary browser sign-in, opened at the address your server gives; its answer comes back to the device itself (`127.0.0.1`) and goes on to your server, which makes the token exchange |
| Tailscale | Apple apps only, and only if you turn on *Connect Through Tailscale* | Tailscale's coordination service, as with any Tailscale device |
| The demo's hosts (`archive.org`, `download.blender.org`, `test-streams.mux.dev`) | Apple apps only, and only in *Try the Demo* | ordinary video requests for the demo films |

Device names ("iPhone", "Apple TV") are sent to your server for its device list. Nothing is sent
to the developers of Moonrise.

## App Store privacy label

"Data Not Collected": the developer collects nothing. (Data sent to the user's own server, and to
the services above at the user's request, is not collection by the developer under Apple's
definitions; TMDB, OpenSubtitles, OMDb, IMDb and segment-database requests are made by the user's
own server.)
