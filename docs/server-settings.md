# Server settings

Everything is set in the app (**Settings → Server → Manage Server**, admins only) and kept in the
server's database (`/data/moonrise.db`). Environment variables still work and **win** over what the
app stores; the app then shows the value as "set by the environment" and can't change it. The
full reference, with every API, is in [docs/dev/server.md](dev/server.md#settings).

## In the app

| Screen | Settings |
|---|---|
| Libraries | Each library's folders (on the server, or on SMB shares), Scan Now |
| Fix Match | Titles the server couldn't recognise for sure |
| Users / Invitations | Accounts, admins, signed-in devices, one-time invitation codes |
| Services | TMDB key and metadata language/country, OpenSubtitles account, AI Matching (the LLM: provider, address, model, key, Test, Sign in with ChatGPT) |
| Playback & Hardware | Transcoding hardware, jobs at once, cache and download space, scan intervals, Prepare Files Ahead |
| Activity / Logs / About | What plays now, background jobs, the last log lines, restart |

## Environment variables

| Variable | Setting | Default |
|---|---|---|
| `MOONRISE_LISTEN` | (environment only) address and port | `0.0.0.0:8097` |
| `MOONRISE_DATA_DIR`, `MOONRISE_CACHE_DIR` | (environment only) state and cache folders | `/data`, `/cache` (the compose example puts the cache in `/data/cache`) |
| `MOONRISE_NAME` | `server.name` | Moonrise Server |
| `MOONRISE_TMDB_LANGUAGE`, `MOONRISE_TMDB_COUNTRY` | `metadata.language`, `.country` | `en`, `US` |
| `TMDB_API_KEY` | `tmdb.api_key` | the built-in key (release builds) |
| `OPENSUBTITLES_API_KEY`, `_USERNAME`, `_PASSWORD` | `opensubtitles.*` | the built-in key, anonymous |
| `MOONRISE_LLM_ENABLED`, `MOONRISE_LLM_PROVIDER` (older name `MOONRISE_LLM_API_STYLE`), `MOONRISE_LLM_BASE_URL`, `MOONRISE_LLM_MODEL`, `MOONRISE_LLM_API_KEY`, `MOONRISE_LLM_OUTPUT` | `llm.*` (see [AI matching](#ai-matching)) | `anthropic`; off without a key |
| `MOONRISE_HWACCEL`, `MOONRISE_HWACCEL_DEVICE` | `transcode.hardware`, `.device` | `auto` |
| `MOONRISE_TRANSCODE_JOBS`, `MOONRISE_CONVERT_JOBS`, `MOONRISE_JOBS`, `MOONRISE_DOWNLOAD_JOBS` | `performance.*` | 2, 4, 4, 1 |
| `MOONRISE_CACHE_MAX_GB`, `MOONRISE_DOWNLOAD_MAX_GB` | `performance.cache_max_gb`, `.download_max_gb` | 10, 12 |
| `MOONRISE_SCAN_QUICK_SECS`, `MOONRISE_SCAN_FULL_HOURS` | `performance.scan_*` | 120, 6 |
| `MOONRISE_SMB_SIGNING` | `smb.signing` | `auto` |
| `MOONRISE_DISABLE` | `features.*` (comma-separated switches, e.g. `transcode,mdns`) | none |
| `MOONRISE_RESTART=exit` | (environment only) lets the app's Restart Server exit the server for Docker to start again | unset |
| `MOONRISE_MDNS_INTERFACES`, `MOONRISE_BROWSE_ROOTS` | (environment only) Bonjour interfaces, folders offered by the folder picker | automatic |
| `RUST_LOG` | (environment only) log level | `moonrise_server=info` |

## Preparing files ahead

To cut a title into segments the server needs the times of its keyframes. Matroska (MKV) and MP4
files keep them in an index the server reads in a fraction of a second. AVI, MPEG-TS/PS and
similar files don't: the server reads the whole file once to find them (a few seconds for an
episode, up to a minute for a large film on a network share), and the first play of such a file
waits for that. The result is kept, so later plays start at once.

**Prepare Files Ahead** (on by default; **Playback & Hardware**) does this in the background
before anyone presses Play: for new and changed files after each library scan, and once for every
file that has no index yet. The titles being watched go first (resume points, the next episodes
of shows in progress), then the newest files. It reads one file at a time, at about 10 MB/s from a
network share, at low CPU priority, and stops whenever anything is playing, being transcoded or
downloaded, carrying on where it was when that ends; a title played while it is being prepared
is finished at full speed. Progress shows in Playback & Hardware and in Activity ("Preparing
files: 120 of 281"). After a restart it carries on with the files left. Turned off, a file is
prepared when it is first played, as before.

| Where | Setting | Default |
|---|---|---|
| App | Playback & Hardware → Prepare Files Ahead | on |
| Settings API | `features.index_backfill` (`true`/`false`) | `true` |
| Environment | `MOONRISE_DISABLE=index_backfill` (wins over the app) | not set |

## Command line

Inside the container (`docker exec -it moonrise-server moonrise-server …`): `user add <name> [--admin]`
(password on stdin; the first account claims a new server), `user passwd`, `user list`,
`scan [--full]`, `stats`, `llm-test`, `llm-replay [<n>]`, `smb-credential add`, `migrate-root`, `healthcheck`.

## AI matching

The server asks a language model only about file names it can't match by name, year and runtime,
and checks every answer (an answer that doesn't check out goes to Fix Match). Choose the provider
in **Services → AI Matching**, or with `MOONRISE_LLM_PROVIDER`:

| Provider | `llm.provider` | Address (`llm.base_url`) | Default model | Key | Answer format |
|---|---|---|---|---|---|
| Anthropic | `anthropic` | `https://api.anthropic.com` | `claude-haiku-5-5` | API key (`ANTHROPIC_API_KEY`) | JSON schema |
| OpenAI | `openai` | `https://api.openai.com/v1` | `gpt-5.4-mini` | API key (`OPENAI_API_KEY`) | JSON schema |
| ChatGPT plan | `chatgpt` | `https://api.openai.com/v1` (Responses) | the account's first model | Sign in with ChatGPT (in the app) | Automatic |
| Google Gemini | `gemini` | `https://generativelanguage.googleapis.com/v1beta/openai` | `gemini-2.5-flash` | API key (`GEMINI_API_KEY`) | Automatic |
| OpenRouter | `openrouter` | `https://openrouter.ai/api/v1` | `google/gemini-2.5-flash` | API key (`OPENROUTER_API_KEY`) | Automatic |
| Ollama | `ollama` | `http://localhost:11434/v1` | choose one | none | Automatic |
| LM Studio | `lmstudio` | `http://localhost:1234/v1` | choose one | none | Automatic |
| llama.cpp | `llamacpp` | `http://localhost:8080/v1` | the one it loaded | none | Automatic |
| Other OpenAI-compatible (vLLM, LocalAI, a gateway…) | `custom` | required | required | optional | Automatic |

- **Address:** "localhost" is the server's own container. For a model server on another machine
  (or the Docker host) use its LAN address, e.g. `http://192.168.1.20:11434/v1`.
- **Answer format** (`llm.output`, `MOONRISE_LLM_OUTPUT`): `schema` (structured output), `tool` (a
  forced tool call), `prompt` (JSON asked for in the prompt, read leniently) or `auto`: the first
  call tries them in that order and keeps the first that works. Servers without structured
  output or tool calls still work through `prompt`. A reply that isn't the JSON asked for is
  never guessed at: the title waits in Fix Match.
- **Test** runs one real identification of a sample file with the settings on screen, before they
  are saved, and says whether the model got it right. **Choose a Model** lists what the endpoint
  offers (`/models`). `moonrise-server llm-replay 20` asks the configured model the last 20
  questions kept in the database and compares its answers with the ones kept (to try another
  model on your own library; nothing is written).
- **Keys:** a key is kept for the provider and the address (scheme, host and port) it was
  saved with, and is only ever sent there: every request the server makes is checked against
  that, and a request to anywhere else goes without it. Switching providers puts the old one's
  model, address and format aside and brings them back when it is chosen again, with its key.
  Changing a provider's address sets its key aside (it isn't deleted): enter a key for the new
  address, or go back to the old address and the key comes back. A settings change that names the
  same setting twice (say `{"llm": {"base_url": …}, "llm.base_url": …}`) is refused. Redirects are
  never followed when a key or header is sent: give the address the requests should go to.
  Keys are never shown again, never logged, and cut out of error messages. In the environment, the
  key comes from `MOONRISE_LLM_API_KEY`, else the provider's own variable in the table (or the one
  `MOONRISE_LLM_API_KEY_ENV` names). A key (and `MOONRISE_LLM_EXTRA_HEADERS`) from the
  environment is only used for the provider and address the environment names
  (`MOONRISE_LLM_PROVIDER`, default Anthropic, and `MOONRISE_LLM_BASE_URL` or the provider's
  address; a provider's own variable, such as `OPENROUTER_API_KEY`, goes with that provider):
  pick another provider or address in the app and give it its own key there.
- **Sign in with ChatGPT** uses your ChatGPT plan's included usage instead of an API key
  (OpenAI's flow for open-source and self-hosted apps). The app opens OpenAI's sign-in page and
  receives the answer on the device (`http://127.0.0.1:1455`), then hands it to the server, which
  checks the sign-in (its ID token is verified against OpenAI's published keys), keeps the tokens
  (in its database, never shown) and renews them. They are only ever sent to OpenAI: the ChatGPT
  plan is always used at `https://api.openai.com/v1`, and no other address can be set for it. In
  ChatGPT's settings you can set a weekly limit for Moonrise or disconnect it; past the limit,
  unclear titles wait in Fix Match until it resets. Sign in on iPhone, iPad or Mac (not Apple TV).
  **Sign Out** ends the sign-in at OpenAI first (it revokes the renewal token), then forgets the
  tokens; if OpenAI can't confirm it, the app says so: disconnect Moonrise in ChatGPT's settings
  to be sure. Moonrise stays registered with that account, so signing in again goes straight back
  to it (a different account answering is refused). **Use Another Account** registers Moonrise
  for another account as a separate registration: each account keeps its own registration and
  sign-in, and switching back to one signs in again with its registration rather than making a
  new one. Sign Out applies to the account in use; the others are left as they are.
  If OpenAI turns down the sign-in's one-time code (`invalid_grant`), Moonrise keeps the
  registration OpenAI just made, and the app signs in once more by itself with it, in a private
  browser session (no cookies shared with Safari), without registering Moonrise again. Only that
  one retry uses the kept registration: if it fails too, or the sign-in fails in any other way
  (for example `invalid_client`), the unfinished registration is dropped, and the next Sign In
  or Use Another Account registers Moonrise afresh. An unfinished registration that is left over
  (the retry was cancelled, say) shows in AI Matching with **Forget**; ChatGPT's settings may
  list it as well, and it can be removed there. Apps from before server 0.22.2 don't sign in
  again by themselves: for them a turned-down code drops the registration at once, and the
  message asks to update the app. The server log has a line for each step (start, callback,
  token request with OpenAI's error and request id, an error OpenAI sent back to the app), each
  naming the app's sign-in flow (`app="flow 2"`; an older app shows as such), never codes or
  tokens: the callback line describes the code by its length, character classes and parts
  (`1903 chars, no ac_ prefix, chars [A-Z a-z 0-9 - _ .], 3 parts (…)`).
  OpenAI's sign-in page lists the host id (`ext_agent_host_id`) as a required host identifier,
  but OpenAI's own reference client (`sign-in-with-chatgpt-devkit`) leaves it out unless its
  `sendHostId` option is on, which is off by default. Moonrise matches the reference client: the
  host id is sent only with `MOONRISE_CHATGPT_SEND_HOST_ID=1`.
- **Subscription sign-in elsewhere:** Anthropic's and Google's consumer plans (Claude Pro/Max,
  Gemini) can't be used by other apps: use an API key (Google AI Studio has a free tier).
  OpenRouter works with an ordinary API key.

## Backups

Back up the data folder (`/data`): `moonrise.db` holds the settings (including keys and share
passwords: it is readable by its owner only), accounts, watch history and the library;
`subs/` holds subtitles fetched from OpenSubtitles. `cache/` can be left out.
