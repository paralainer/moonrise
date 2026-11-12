//! Configuration: the settings store in the database (`settings.rs`, edited through the admin
//! API) with environment variables on top.
//!
//! **Precedence**, per setting: an environment variable that is set (and not empty) wins, then
//! the value stored in the database, then (for the TMDB key and the OpenSubtitles consumer key)
//! the project's key compiled into a release build ([`Builtin`]), then the built-in default. `MOONRISE_DISABLE` can only switch
//! features off (a feature it lists is off whatever the store says). OpenSubtitles credentials
//! in `MOONRISE_OPENSUBTITLES_ENV` (default `<data dir>/secrets/opensubtitles.env`) count as
//! environment. Some settings exist only in the environment (where the server listens, its
//! directories, ffmpeg, the `db` and `moonrise_auth` switches, the LLM's advanced knobs).
//!
//! `Debug` output is safe to log: secret values are wrapped in [`Secret`], which prints as
//! `***`.

use std::{
    collections::{BTreeSet, HashMap},
    fmt,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, RwLock},
};

use crate::admin::settings::Stored;

#[derive(Clone, Debug)]
pub struct Config {
    pub listen: SocketAddr,
    /// Shown in `/v1/info`.
    pub name: String,
    pub cache_dir: PathBuf,
    pub cache_max_bytes: u64,
    /// Persistent data: the SQLite database (`moonrise.db`).
    pub data_dir: PathBuf,
    pub ffmpeg: String,
    pub ffprobe: String,
    /// Concurrent ffmpeg jobs.
    pub jobs: usize,
    /// Concurrent audio conversion runs (`MOONRISE_CONVERT_JOBS`).
    pub convert_jobs: usize,
    /// Concurrent video transcode runs on the GPU (`MOONRISE_TRANSCODE_JOBS`).
    pub transcode_jobs: usize,
    /// Concurrent prepared-download transcodes on the GPU (`MOONRISE_DOWNLOAD_JOBS`), on top of
    /// the playback runs.
    pub download_jobs: usize,
    /// Disk space for prepared downloads (`MOONRISE_DOWNLOAD_MAX_GB`), apart from the segment cache.
    pub download_max_bytes: u64,
    /// Disk space for scrubbing thumbnails (`MOONRISE_TRICKPLAY_MAX_GB`), apart from the
    /// segment cache.
    pub trickplay_max_bytes: u64,
    /// OpenSubtitles.com credentials: `OPENSUBTITLES_API_KEY`, `OPENSUBTITLES_USERNAME`,
    /// `OPENSUBTITLES_PASSWORD`, from the environment or from the file
    /// `MOONRISE_OPENSUBTITLES_ENV` (default `/data/secrets/opensubtitles.env`, `KEY=value` lines,
    /// read literally).
    pub opensubtitles: OpenSubtitlesCreds,
    /// Kill switches: `MOONRISE_DISABLE=db,probe_cache,convert,mp4_index,scan_index,transcode,burn,
    /// subs_extract,downloads,opensubtitles,library,llm,moonrise_auth,mdns`, and the store's
    /// `features.*`. See [`Feature`]. Shared by the clones of one `Config`, so the switches that
    /// apply live are changed for every holder at once.
    pub disabled: Switches,
    /// `MOONRISE_LIBRARIES` (`Name=kind:/root[,/root2];…`, kind `movies` or `shows`), else the
    /// store's libraries; none by default.
    pub libraries: Vec<LibraryConfig>,
    /// Quick scan (directory mtimes) every `MOONRISE_SCAN_QUICK_SECS` (120), a full walk every
    /// `MOONRISE_SCAN_FULL_HOURS` (6).
    pub scan_quick_secs: u64,
    pub scan_full_hours: u64,
    /// Files probed at once by the scanner (`MOONRISE_SCAN_PROBE_JOBS`, 1).
    pub scan_probe_jobs: usize,
    /// `TMDB_API_KEY`.
    pub tmdb_key: Secret,
    /// `OMDB_API_KEY` (setting `omdb.api_key`): Rotten Tomatoes and Metacritic scores
    /// (`library/ratings.rs`). Empty: IMDb's own dataset only.
    pub omdb_key: Secret,
    /// TMDB metadata language and certification country (`MOONRISE_TMDB_LANGUAGE` en,
    /// `MOONRISE_TMDB_COUNTRY` US).
    pub tmdb_language: String,
    pub tmdb_country: String,
    pub llm: LlmConfig,
    /// `MOONRISE_BROWSE_ROOTS` (comma-separated): the folders the admin API may list for picking
    /// library roots; empty: every top-level folder but the system's and the server's own.
    pub browse_roots: Vec<String>,
    /// SMB signing (`MOONRISE_SMB_SIGNING`, setting `smb.signing`): `auto` signs only when the server
    /// requires it, `always` every session.
    pub smb_signing: String,
    /// The SMB shares the libraries use, with their credentials (from the store).
    pub smb_shares: Vec<crate::storage::smb::ShareConfig>,
    /// What transcodes video (`MOONRISE_HWACCEL`, setting `transcode.hardware`): auto, nvidia, qsv,
    /// vaapi, videotoolbox, v4l2 or cpu (`hwaccel.rs`); `MOONRISE_HWACCEL_DEVICE`
    /// (`transcode.device`): a render node.
    pub hwaccel: String,
    pub hwaccel_device: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LibraryConfig {
    pub id: String,
    pub name: String,
    /// "movies", "shows" or "mixed": what it was made as, for older apps; never what its files
    /// are taken for (see `library::parse`).
    pub kind: String,
    /// Local directories, and SMB shares as `smb://host/share/path` (the store's roots).
    pub roots: Vec<String>,
}

/// The LLM used for ambiguous identifications (see `llm/mod.rs`).
#[derive(Clone, Debug, PartialEq)]
pub struct LlmConfig {
    /// In use: allowed, and a key resolves (or an openai-style endpoint that needs none).
    pub enabled: bool,
    /// `MOONRISE_LLM_ENABLED`: `auto` (true: used when it can be) or `off` (false).
    pub allowed: bool,
    /// `MOONRISE_LLM_PROVIDER` (or `MOONRISE_LLM_API_STYLE`): a preset of `llm/mod.rs`
    /// (`anthropic`, the default; `openai`, `chatgpt`, `gemini`, `openrouter`, `ollama`,
    /// `lmstudio`, `llamacpp`, `custom`). An unknown name is taken as `custom`.
    pub provider: String,
    /// The preset's API style: `anthropic`, `openai` (Chat Completions) or `responses`.
    pub style: String,
    /// `MOONRISE_LLM_BASE_URL`, default the preset's (`custom` has none: required).
    pub base_url: String,
    /// `MOONRISE_LLM_MODEL`, default the preset's (`claude-haiku-5-5` for anthropic).
    pub model: String,
    /// From `MOONRISE_LLM_API_KEY`, else the variable named by `MOONRISE_LLM_API_KEY_ENV`
    /// (default the preset's: `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`,
    /// `OPENROUTER_API_KEY`; none for the others).
    pub api_key: Secret,
    /// Where `api_key` may be sent (`llm/keys.rs`): the request layer attaches it only to a
    /// request of this provider at this origin. None: nowhere.
    pub key_for: Option<crate::llm::keys::Binding>,
    /// `MOONRISE_LLM_OUTPUT`: `auto`, `schema`, `tool` or `prompt`; default the preset's.
    pub output: String,
    /// `MOONRISE_LLM_ANTHROPIC_VERSION`, default `2026-10-21`.
    pub anthropic_version: String,
    /// `MOONRISE_LLM_EFFORT` (anthropic `output_config.effort`), default `low`; empty: not sent.
    pub effort: String,
    pub max_tokens: u32,
    pub timeout_secs: u64,
    /// `MOONRISE_LLM_MAX_CALLS_PER_SCAN` (200): over it, items go to review.
    pub max_calls_per_scan: usize,
    /// `MOONRISE_LLM_VOTES` (1): 3 asks three times for low-confidence verdicts, 2 of 3 must agree.
    pub votes: usize,
    /// `MOONRISE_LLM_EXTRA_HEADERS`: `Name: value; …` (values are treated as secrets: not logged).
    pub extra_headers: Vec<(String, Secret)>,
    /// Where `extra_headers` may be sent, as `key_for`.
    pub headers_for: Option<crate::llm::keys::Binding>,
}

impl LlmConfig {
    /// Whether it can be used: allowed, an address and a model, and a key where one is needed
    /// (the ChatGPT plan: signed in, see `Config::resolve`).
    fn usable(&self, signed_in: bool) -> bool {
        let preset = crate::llm::preset_or_custom(&self.provider);
        if preset.key == crate::llm::KeyNeed::SignIn {
            return self.allowed && !self.model.is_empty() && signed_in;
        }
        // A key is needed at the provider's own address; another address (a proxy, a local
        // server) may need none. llama.cpp serves the model it loaded, whatever the request names.
        let needs_key =
            preset.key == crate::llm::KeyNeed::Required && crate::llm::origin(&self.base_url) == crate::llm::origin(preset.base_url);
        self.allowed
            && !self.base_url.is_empty()
            && (!self.model.is_empty() || preset.id == "llamacpp")
            && (!self.api_key.expose().is_empty() || !needs_key)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OpenSubtitlesCreds {
    pub api_key: Secret,
    pub username: Secret,
    pub password: Secret,
}

impl OpenSubtitlesCreds {
    /// A consumer key: search works, downloads too (anonymous: a few a day per IP address).
    pub fn usable(&self) -> bool {
        !self.api_key.expose().is_empty()
    }

    /// An account too: downloads count against the account's daily quota.
    pub fn has_account(&self) -> bool {
        self.usable() && !self.username.expose().is_empty() && !self.password.expose().is_empty()
    }
}

/// Capabilities that can be switched off without a rebuild (`MOONRISE_DISABLE`, comma-separated).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feature {
    /// The SQLite database. Off: no persistent state; probe cache off; `server_id` changes on
    /// every start.
    Db,
    /// Reuse ffprobe output and the keyframe index across restarts (stored in the database).
    ProbeCache,
    /// Audio that can't be copied (DTS, TrueHD, MP3, Vorbis, Opus, FLAC, PCM…) is converted to
    /// AAC or E-AC-3. Off: such tracks aren't offered.
    Convert,
    /// The keyframe index of MP4/MOV files, read from their `moov` box. Off: the background
    /// packet scan is used for them too.
    Mp4Index,
    /// The keyframe index of other containers (AVI, MPEG-PS/TS…): a one-time background packet
    /// scan with ffprobe, kept in the database. Off: such files aren't served.
    ScanIndex,
    /// Video transcoding on the GPU (NVENC): the quality ladder, codecs that can't be copied
    /// (MPEG-4 Part 2, MPEG-2, AV1, VC-1, 10-bit H.264), HDR tone-mapped for SDR displays. Off:
    /// only the copied original is served.
    Transcode,
    /// Bitmap subtitles (PGS, DVD) burned into a transcoded variant (`burn=`). Off: such
    /// requests are answered `supported: false`.
    Burn,
    /// Text subtitles extracted here (one ffmpeg pass per file, kept in `/data/subs`) and
    /// sidecar files read from the media share. Off: text subtitles aren't served.
    SubsExtract,
    /// Subtitle windows (`GET /v1/items/{id}/subtitles/{index}/cues`): a track's cues for 30 s
    /// read on their own while the whole track isn't extracted yet; the HLS rendition's segments
    /// come from them too. Off: not announced (apps fetch whole tracks), segments wait ≤ 6 s
    /// for the extraction.
    SubsWindows,
    /// The library backfill: every title's text subtitles extracted ahead of need, the next
    /// episodes of what plays first. Off: only what plays (and downloads) is extracted.
    SubsBackfill,
    /// Downloaded (fetched or uploaded) subtitles matched to the file's timing (`subsync.rs`).
    /// Off: they are served as they are.
    SubsSync,
    /// Keyframe indexes made ahead of need (`index_backfill.rs`): new and changed files, and once
    /// every file without one, in the background at a pace, stopped while anything plays. Off:
    /// a file's index is made when it is first played (a scanned file waits for its scan then).
    IndexBackfill,
    /// End credits looked for in an episode's file (`credits.rs`): the last minutes' keyframes,
    /// once per file, when it is first played. Off: the apps go by chapters and the community
    /// databases only.
    CreditsDetect,
    /// `GET /v1/items/{id}/download` and prepared MP4 downloads. Off: 503.
    Downloads,
    /// Subtitle search and fetch through OpenSubtitles.com. Off: 503; subtitles fetched before
    /// stay listed.
    OpenSubtitles,
    /// The library: scanning the shares, identification, TMDB metadata and artwork, and the
    /// browse/search API. Off: no background scans; the browse API answers 503.
    Library,
    /// The LLM step of identification. Off: ambiguous files go to review.
    Llm,
    /// Moonrise's own accounts (Phase 3): `Authorization: Bearer` tokens, sign-in and device-code
    /// endpoints, `/link`, watch state and playback reporting. Off: Bearer tokens are refused
    /// and those endpoints answer 503 (only the local admin token works).
    MoonriseAuth,
    /// Advertising the server on the local network as Bonjour `_moonrise._tcp` (mDNS), so apps find
    /// it without an address. Off: apps find it by probing the subnet (`GET /v1/info`).
    Mdns,
    /// Ratings beside TMDB's (`library/ratings.rs`): IMDb's from its public dataset, Rotten
    /// Tomatoes and Metacritic through OMDb with a key. Off: nothing is fetched or answered.
    Ratings,
    /// Scrubbing thumbnails made ahead of need (`trickplay.rs`): a keyframe every 10 s of every
    /// title, in the background, stopped while anything plays. Off: nothing more is made, and
    /// the apps show no thumbnails.
    Trickplay,
    /// Intro, recap and end-credits times from the community databases TheIntroDB and IntroDB
    /// (`library/community.rs`), asked by the server when a title is opened or played, kept in the
    /// database, and merged into `segments?merged=1`. Off: nothing is asked; the segments come
    /// from the files alone (chapters, end credits found in the file).
    CommunitySegments,
}

impl Feature {
    pub const ALL: [Feature; 22] = [
        Feature::Db,
        Feature::ProbeCache,
        Feature::Convert,
        Feature::Mp4Index,
        Feature::ScanIndex,
        Feature::Transcode,
        Feature::Burn,
        Feature::SubsExtract,
        Feature::SubsWindows,
        Feature::SubsBackfill,
        Feature::SubsSync,
        Feature::IndexBackfill,
        Feature::CreditsDetect,
        Feature::Downloads,
        Feature::OpenSubtitles,
        Feature::Library,
        Feature::Llm,
        Feature::MoonriseAuth,
        Feature::Mdns,
        Feature::Ratings,
        Feature::Trickplay,
        Feature::CommunitySegments,
    ];

    pub fn from_key(k: &str) -> Option<Feature> {
        Feature::ALL.into_iter().find(|f| f.key() == k)
    }

    /// Only the environment can switch it (off in the store it would lock the admin out, or
    /// the store itself would be gone).
    pub fn env_only(self) -> bool {
        matches!(self, Feature::Db | Feature::MoonriseAuth)
    }

    /// Switching it takes effect at once; the others when the server restarts (they decide
    /// what is set up at start).
    pub fn live(self) -> bool {
        !matches!(
            self,
            Feature::Db | Feature::MoonriseAuth | Feature::Library | Feature::SubsExtract | Feature::SubsBackfill | Feature::Transcode
        )
    }

    pub fn key(self) -> &'static str {
        match self {
            Feature::Db => "db",
            Feature::ProbeCache => "probe_cache",
            Feature::Convert => "convert",
            Feature::Mp4Index => "mp4_index",
            Feature::ScanIndex => "scan_index",
            Feature::Transcode => "transcode",
            Feature::Burn => "burn",
            Feature::SubsExtract => "subs_extract",
            Feature::SubsWindows => "subs_windows",
            Feature::SubsBackfill => "subs_backfill",
            Feature::SubsSync => "subs_sync",
            Feature::IndexBackfill => "index_backfill",
            Feature::CreditsDetect => "credits_detect",
            Feature::Downloads => "downloads",
            Feature::OpenSubtitles => "opensubtitles",
            Feature::Library => "library",
            Feature::Llm => "llm",
            Feature::MoonriseAuth => "moonrise_auth",
            Feature::Mdns => "mdns",
            Feature::Ratings => "ratings",
            Feature::Trickplay => "trickplay",
            Feature::CommunitySegments => "community_segments",
        }
    }
}

/// The LLM's credentials and where they may go (`llm/keys.rs`). A key or extra headers from the
/// environment are bound to the provider and address they were given for there: the
/// environment's provider (`MOONRISE_LLM_PROVIDER`, default `anthropic`; for a preset's own
/// variable such as `OPENROUTER_API_KEY`, that preset) at the environment's
/// `MOONRISE_LLM_BASE_URL` or the preset's address. Otherwise the key is the one the settings
/// keep for this provider at this address, if any. A key bound elsewhere isn't used.
fn llm_credentials(llm: &mut LlmConfig, env: &Env, store: &Stored) {
    use crate::llm::keys::Binding;
    let env_provider = env
        .get("MOONRISE_LLM_PROVIDER")
        .or_else(|| env.get("MOONRISE_LLM_API_STYLE"))
        .map_or("anthropic", |p| {
            crate::llm::preset_or_custom(p.trim().to_ascii_lowercase().as_str()).id
        });
    let env_binding = |owner: &str| {
        let base = env
            .get("MOONRISE_LLM_BASE_URL")
            .map_or_else(|| crate::llm::preset_or_custom(owner).base_url.to_string(), String::from);
        Binding::new(owner, &base)
    };
    if let ("env" | "file", Some(var)) = crate::admin::settings::source(env, store, "llm.api_key") {
        let own_var = env.get("MOONRISE_LLM_API_KEY_ENV").is_none() && var == crate::llm::preset_or_custom(&llm.provider).key_env;
        let owner = if own_var { llm.provider.clone() } else { env_provider.to_string() };
        llm.key_for = env_binding(&owner);
    } else {
        match crate::llm::keys::find(store, &llm.provider, &llm.base_url) {
            Some((key, b)) => (llm.api_key, llm.key_for) = (key, Some(b)),
            None => (llm.api_key, llm.key_for) = (Secret::default(), None),
        }
    }
    if !llm.key_for.as_ref().is_some_and(|b| b.allows_str(&llm.provider, &llm.base_url)) {
        llm.api_key = Secret::default();
        llm.key_for = None;
    }
    llm.headers_for = if llm.extra_headers.is_empty() {
        None
    } else {
        env_binding(env_provider)
    };
    if !llm.headers_for.as_ref().is_some_and(|b| b.allows_str(&llm.provider, &llm.base_url)) {
        llm.extra_headers.clear();
        llm.headers_for = None;
    }
    llm.enabled = llm.usable(crate::llm::chatgpt::signed_in(store.get(crate::llm::chatgpt::TOKENS_KEY)));
}

impl Config {
    /// The effective configuration: `env` over `store` over the defaults.
    pub fn resolve(env: &Env, store: &Stored) -> Self {
        let mut c = Self::from_lookup(|k| {
            env.get(k)
                .map(String::from)
                // A stored key is bound to where it may go: `llm_credentials`, never this.
                .or_else(|| {
                    if k == "MOONRISE_LLM_API_KEY" || crate::admin::settings::env_sets(env, store, k) {
                        None
                    } else {
                        store.env_value(k)
                    }
                })
                .or_else(|| env.builtin.var(k).map(String::from))
        });
        if env.get("MOONRISE_LIBRARIES").is_none() {
            c.libraries = store.libraries().iter().map(super::settings::LibrarySetting::runtime).collect();
        }
        c.smb_shares = crate::admin::settings::smb_shares(&c.libraries, store);
        let mut off = c.disabled.get();
        for f in Feature::ALL {
            if !f.env_only() && store.feature(f) == Some(false) {
                off.insert(f.key().to_string());
            }
        }
        c.disabled = Switches::new(off);
        llm_credentials(&mut c.llm, env, store);
        c
    }

    /// Built from one lookup (environment-style names and values).
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Self {
        let env = |k: &str, d: &str| get(k).unwrap_or_else(|| d.to_string());
        Config {
            listen: env("MOONRISE_LISTEN", "0.0.0.0:8097").parse().expect("MOONRISE_LISTEN"),
            name: env("MOONRISE_NAME", "Moonrise Server"),
            cache_dir: env("MOONRISE_CACHE_DIR", &default_dir("cache")).into(),
            cache_max_bytes: env("MOONRISE_CACHE_MAX_GB", "10").parse::<u64>().expect("MOONRISE_CACHE_MAX_GB") << 30,
            data_dir: env("MOONRISE_DATA_DIR", &default_dir("data")).into(),
            ffmpeg: env("MOONRISE_FFMPEG", &default_tool("ffmpeg")),
            ffprobe: env("MOONRISE_FFPROBE", &default_tool("ffprobe")),
            jobs: env("MOONRISE_JOBS", "4").parse().expect("MOONRISE_JOBS"),
            convert_jobs: env("MOONRISE_CONVERT_JOBS", "4").parse().expect("MOONRISE_CONVERT_JOBS"),
            transcode_jobs: env("MOONRISE_TRANSCODE_JOBS", "2").parse().expect("MOONRISE_TRANSCODE_JOBS"),
            download_jobs: env("MOONRISE_DOWNLOAD_JOBS", "1").parse().expect("MOONRISE_DOWNLOAD_JOBS"),
            download_max_bytes: env("MOONRISE_DOWNLOAD_MAX_GB", "12")
                .parse::<u64>()
                .expect("MOONRISE_DOWNLOAD_MAX_GB")
                << 30,
            trickplay_max_bytes: env("MOONRISE_TRICKPLAY_MAX_GB", "10")
                .parse::<u64>()
                .expect("MOONRISE_TRICKPLAY_MAX_GB")
                << 30,
            opensubtitles: OpenSubtitlesCreds {
                api_key: Secret::new(get("OPENSUBTITLES_API_KEY").unwrap_or_default()),
                username: Secret::new(get("OPENSUBTITLES_USERNAME").unwrap_or_default()),
                password: Secret::new(get("OPENSUBTITLES_PASSWORD").unwrap_or_default()),
            },
            disabled: Switches::new(
                env("MOONRISE_DISABLE", "")
                    .split(',')
                    .map(|s| s.trim().to_ascii_lowercase())
                    .filter(|s| !s.is_empty())
                    .collect(),
            ),
            libraries: parse_libraries(&env("MOONRISE_LIBRARIES", "")),
            scan_quick_secs: env("MOONRISE_SCAN_QUICK_SECS", "120").parse().expect("MOONRISE_SCAN_QUICK_SECS"),
            scan_full_hours: env("MOONRISE_SCAN_FULL_HOURS", "6").parse().expect("MOONRISE_SCAN_FULL_HOURS"),
            scan_probe_jobs: env("MOONRISE_SCAN_PROBE_JOBS", "1").parse().expect("MOONRISE_SCAN_PROBE_JOBS"),
            tmdb_key: Secret::new(get("TMDB_API_KEY").unwrap_or_default().trim().to_string()),
            omdb_key: Secret::new(get("OMDB_API_KEY").unwrap_or_default().trim().to_string()),
            tmdb_language: env("MOONRISE_TMDB_LANGUAGE", "en"),
            tmdb_country: env("MOONRISE_TMDB_COUNTRY", "US"),
            llm: {
                let provider = get("MOONRISE_LLM_PROVIDER")
                    .or_else(|| get("MOONRISE_LLM_API_STYLE"))
                    .unwrap_or_else(|| "anthropic".into())
                    .trim()
                    .to_ascii_lowercase();
                let preset = crate::llm::preset_or_custom(&provider);
                let key_env = get("MOONRISE_LLM_API_KEY_ENV").unwrap_or_else(|| preset.key_env.to_string());
                let api_key = Secret::new(
                    get("MOONRISE_LLM_API_KEY")
                        .or_else(|| (!key_env.is_empty()).then(|| get(&key_env)).flatten())
                        .unwrap_or_default()
                        .trim()
                        .to_string(),
                );
                // The ChatGPT plan's token only ever goes to OpenAI: its address can't be changed.
                let base_url = match preset.key {
                    crate::llm::KeyNeed::SignIn => preset.base_url.to_string(),
                    _ => get("MOONRISE_LLM_BASE_URL").unwrap_or_else(|| preset.base_url.to_string()),
                };
                let model = get("MOONRISE_LLM_MODEL").unwrap_or_else(|| preset.model.to_string());
                let allowed = env("MOONRISE_LLM_ENABLED", "auto") != "off";
                let extra_headers: Vec<(String, Secret)> = env("MOONRISE_LLM_EXTRA_HEADERS", "")
                    .split(';')
                    .filter_map(|h| h.split_once(':'))
                    .map(|(k, v)| (k.trim().to_string(), Secret::new(v.trim())))
                    .filter(|(k, _)| !k.is_empty())
                    .collect();
                // All from one lookup: bound to where they were given for (`Config::resolve`
                // binds them again with the store).
                let here = crate::llm::keys::Binding::new(preset.id, &base_url);
                let key_for = if api_key.expose().is_empty() { None } else { here.clone() };
                let headers_for = if extra_headers.is_empty() { None } else { here };
                let mut llm = LlmConfig {
                    enabled: false,
                    allowed,
                    provider: preset.id.to_string(),
                    style: preset.style.to_string(),
                    base_url,
                    model,
                    api_key,
                    key_for,
                    output: get("MOONRISE_LLM_OUTPUT").map_or_else(|| preset.output.to_string(), |o| o.trim().to_ascii_lowercase()),
                    anthropic_version: env("MOONRISE_LLM_ANTHROPIC_VERSION", "2026-10-21"),
                    effort: env("MOONRISE_LLM_EFFORT", "low"),
                    max_tokens: env("MOONRISE_LLM_MAX_TOKENS", "2048").parse().expect("MOONRISE_LLM_MAX_TOKENS"),
                    timeout_secs: env("MOONRISE_LLM_TIMEOUT_SECS", "60").parse().expect("MOONRISE_LLM_TIMEOUT_SECS"),
                    max_calls_per_scan: env("MOONRISE_LLM_MAX_CALLS_PER_SCAN", "200")
                        .parse()
                        .expect("MOONRISE_LLM_MAX_CALLS_PER_SCAN"),
                    votes: env("MOONRISE_LLM_VOTES", "1").parse().expect("MOONRISE_LLM_VOTES"),
                    extra_headers,
                    headers_for,
                };
                llm.enabled = llm.usable(false);
                llm
            },
            smb_signing: env("MOONRISE_SMB_SIGNING", "auto").to_ascii_lowercase(),
            hwaccel: env("MOONRISE_HWACCEL", "auto").trim().to_ascii_lowercase(),
            hwaccel_device: env("MOONRISE_HWACCEL_DEVICE", "").trim().to_string(),
            smb_shares: Vec::new(),
            browse_roots: env("MOONRISE_BROWSE_ROOTS", "")
                .split(',')
                .map(|r| r.trim().trim_end_matches('/').to_string())
                .filter(|r| !r.is_empty())
                .collect(),
        }
    }

    pub fn enabled(&self, f: Feature) -> bool {
        !self.disabled.contains(f.key())
    }
}

/// The kill switches that are off, shared by the clones of one [`Config`].
#[derive(Clone, Default)]
pub struct Switches(Arc<RwLock<BTreeSet<String>>>);

impl Switches {
    pub fn new(off: BTreeSet<String>) -> Self {
        Switches(Arc::new(RwLock::new(off)))
    }

    pub fn contains(&self, key: &str) -> bool {
        self.0.read().unwrap().contains(key)
    }

    pub fn get(&self) -> BTreeSet<String> {
        self.0.read().unwrap().clone()
    }

    pub fn set(&self, off: BTreeSet<String>) {
        *self.0.write().unwrap() = off;
    }
}

impl fmt::Debug for Switches {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&*self.0.read().unwrap(), f)
    }
}

/// The environment as the configuration sees it: the process's variables (empty ones count as
/// unset), plus the OpenSubtitles credentials file for the keys the variables don't set.
#[derive(Clone, Debug, Default)]
pub struct Env {
    vars: HashMap<String, Secret>,
    /// Keys that came from the credentials file.
    from_file: BTreeSet<String>,
    /// The project's keys compiled into this build (the last resort for their settings).
    pub builtin: Builtin,
}

/// The project's own keys, compiled into a release build so a new server works without asking
/// for any (as Jellyfin ships its keys): `MOONRISE_BUILTIN_TMDB_KEY` and
/// `MOONRISE_BUILTIN_OPENSUBTITLES_KEY` in the build's environment (the Dockerfile passes them in as
/// build secrets). Empty in source builds. A key in the environment or the store wins; the
/// settings API reports `source: "builtin"` and never shows any of it.
#[derive(Clone, Default, PartialEq)]
pub struct Builtin {
    pub tmdb: Secret,
    pub opensubtitles: Secret,
}

impl Builtin {
    /// The keys of this build.
    pub fn compiled() -> Builtin {
        let get = |v: Option<&'static str>| Secret::new(v.unwrap_or("").trim());
        Builtin {
            tmdb: get(option_env!("MOONRISE_BUILTIN_TMDB_KEY")),
            opensubtitles: get(option_env!("MOONRISE_BUILTIN_OPENSUBTITLES_KEY")),
        }
    }

    /// The built-in value for an environment-style variable, if this build has one.
    pub fn var(&self, var: &str) -> Option<&str> {
        match var {
            "TMDB_API_KEY" => Some(self.tmdb.expose()),
            "OPENSUBTITLES_API_KEY" => Some(self.opensubtitles.expose()),
            _ => None,
        }
        .filter(|v| !v.is_empty())
    }

    /// The built-in value for a setting (`tmdb.api_key`, `opensubtitles.api_key`).
    pub fn for_key(&self, key: &str) -> Option<&str> {
        match key {
            "tmdb.api_key" => self.var("TMDB_API_KEY"),
            "opensubtitles.api_key" => self.var("OPENSUBTITLES_API_KEY"),
            _ => None,
        }
    }
}

impl fmt::Debug for Builtin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Builtin")
            .field("tmdb", &!self.tmdb.expose().is_empty())
            .field("opensubtitles", &!self.opensubtitles.expose().is_empty())
            .finish()
    }
}

/// The data (`data`) or cache (`cache`) folder when `MOONRISE_DATA_DIR`/`MOONRISE_CACHE_DIR` aren't set:
/// the container's volumes `/data` and `/cache`; on macOS the user's Library ("Application
/// Support/Moonrise Server", "Caches/Moonrise Server").
pub fn default_dir(kind: &str) -> String {
    if cfg!(target_os = "macos") {
        if let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
            let home = home.to_string_lossy();
            return match kind {
                "cache" => format!("{home}/Library/Caches/Moonrise Server"),
                _ => format!("{home}/Library/Application Support/Moonrise Server"),
            };
        }
    }
    format!("/{kind}")
}

/// ffmpeg or ffprobe when `MOONRISE_FFMPEG`/`MOONRISE_FFPROBE` aren't set: found on the PATH; on macOS
/// also Homebrew's (a launchd job's PATH has neither /opt/homebrew/bin nor /usr/local/bin).
fn default_tool(name: &str) -> String {
    if cfg!(target_os = "macos") {
        let on_path = std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(name).is_file()));
        if !on_path {
            for dir in ["/opt/homebrew/bin", "/usr/local/bin"] {
                let p = format!("{dir}/{name}");
                if std::path::Path::new(&p).is_file() {
                    return p;
                }
            }
        }
    }
    name.to_string()
}

/// The keys the OpenSubtitles credentials file may hold.
pub const OPENSUBTITLES_VARS: [&str; 3] = ["OPENSUBTITLES_API_KEY", "OPENSUBTITLES_USERNAME", "OPENSUBTITLES_PASSWORD"];

impl Env {
    pub fn from_process() -> Self {
        let mut e = Self::from_pairs(std::env::vars());
        e.builtin = Builtin::compiled();
        let file = e.get("MOONRISE_OPENSUBTITLES_ENV").map_or_else(
            || {
                format!(
                    "{}/secrets/opensubtitles.env",
                    e.get("MOONRISE_DATA_DIR")
                        .map_or_else(|| default_dir("data"), String::from)
                        .trim_end_matches('/')
                )
            },
            String::from,
        );
        if let Ok(text) = std::fs::read_to_string(&file) {
            e.add_file(&text);
        }
        e
    }

    pub fn from_pairs<K: Into<String>, V: Into<String>>(pairs: impl IntoIterator<Item = (K, V)>) -> Self {
        let vars = pairs
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| (k, Secret::new(v)))
            .collect();
        Env {
            vars,
            from_file: BTreeSet::new(),
            builtin: Builtin::default(),
        }
    }

    /// The OpenSubtitles credentials file (`KEY=value` lines): its keys the variables don't set.
    pub fn add_file(&mut self, text: &str) {
        for (k, v) in parse_env_file(text) {
            if OPENSUBTITLES_VARS.contains(&k.as_str()) && !v.is_empty() && !self.vars.contains_key(&k) {
                self.vars.insert(k.clone(), Secret::new(v));
                self.from_file.insert(k);
            }
        }
    }

    pub fn get(&self, k: &str) -> Option<&str> {
        self.vars.get(k).map(Secret::expose)
    }

    /// The variable came from the credentials file, not the process environment.
    pub fn is_from_file(&self, k: &str) -> bool {
        self.from_file.contains(k)
    }
}

/// `Media=/media;Movies=movies:/media/movies;Shows=shows:/media/tv shows` (a kind, `movies`,
/// `shows` or `mixed`, is optional and only kept for older apps). The id is the lower-cased name.
pub fn parse_libraries(s: &str) -> Vec<LibraryConfig> {
    s.split(';')
        .filter_map(|l| {
            let (name, rest) = l.split_once('=')?;
            let (kind, roots) = match rest.split_once(':') {
                Some((k, r)) if ["movies", "shows", "mixed"].contains(&k.trim().to_ascii_lowercase().as_str()) => {
                    (k.trim().to_ascii_lowercase(), r)
                }
                _ => ("mixed".to_string(), rest),
            };
            let name = name.trim().to_string();
            let id = library_id(&name);
            Some(LibraryConfig {
                id,
                name,
                kind,
                roots: roots
                    .split(',')
                    .map(|r| r.trim().trim_end_matches('/').to_string())
                    .filter(|r| !r.is_empty())
                    .collect(),
            })
        })
        .collect()
}

/// A library's id from its name: lower-cased, anything but letters and digits as `-`.
pub fn library_id(name: &str) -> String {
    name.trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect()
}

/// `KEY=value` lines; values are taken literally (no quotes, no interpolation), blank lines and
/// `#` comments skipped.
pub fn parse_env_file(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('=').map(|(k, v)| (k.trim().to_string(), v.to_string())))
        .collect()
}

/// A secret value (API key, password). `Debug` and `Display` never show it.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(s: impl Into<String>) -> Self {
        Secret(s.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.0.is_empty() { "<unset>" } else { "***" })
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_is_redacted() {
        #[derive(Debug)]
        #[allow(dead_code, reason = "the field is only read through Debug")]
        struct WithKey {
            key: Secret,
        }
        let s = Secret::new("sk-ant-very-secret");
        assert_eq!(format!("{s:?}"), "***");
        assert_eq!(format!("{s}"), "***");
        assert_eq!(s.expose(), "sk-ant-very-secret");
        assert_eq!(format!("{:?}", Secret::default()), "<unset>");
        assert!(!format!("{:?}", WithKey { key: s }).contains("very-secret"));
    }

    #[test]
    fn defaults_and_kill_switches() {
        let c = Config::from_lookup(|k| (k == "MOONRISE_DISABLE").then(|| " Probe_Cache , ,x".to_string()));
        assert_eq!(c.data_dir, PathBuf::from(default_dir("data")));
        if !cfg!(target_os = "macos") {
            assert_eq!(c.data_dir, PathBuf::from("/data"));
        }
        assert_eq!(c.cache_max_bytes, 10 << 30);
        assert!(!c.enabled(Feature::ProbeCache));
        assert!(c.enabled(Feature::Db));
        assert!(c.disabled.contains("x"));
        assert_eq!(c.download_jobs, 1);
        assert!(c.libraries.is_empty(), "no libraries until some are configured");
        assert_eq!((c.tmdb_language.as_str(), c.tmdb_country.as_str()), ("en", "US"));
        assert!(!c.llm.enabled, "no key");
        assert!(c.llm.allowed);
        let c = Config::from_lookup(|k| {
            (k == "MOONRISE_LIBRARIES").then(|| "Movies=movies:/media/movies;Shows=shows:/media/tv shows/".to_string())
        });
        assert_eq!(
            c.libraries[1],
            LibraryConfig {
                id: "shows".into(),
                name: "Shows".into(),
                kind: "shows".into(),
                roots: vec!["/media/tv shows".into()]
            }
        );
    }

    #[test]
    fn switches_are_shared_by_clones() {
        let c = Config::from_lookup(|_| None);
        let d = c.clone();
        c.disabled.set(["llm".to_string()].into());
        assert!(!d.enabled(Feature::Llm));
        assert_eq!(Feature::from_key("subs_windows"), Some(Feature::SubsWindows));
        assert!(Feature::ALL.iter().all(|f| Feature::from_key(f.key()) == Some(*f)));
    }

    #[test]
    fn llm_key_by_reference() {
        let c = Config::from_lookup(|k| match k {
            "MOONRISE_LLM_API_KEY_ENV" => Some("MY_KEY".into()),
            "MY_KEY" => Some("sk-secret".into()),
            "TMDB_API_KEY" => Some("tmdb-secret".into()),
            _ => None,
        });
        assert!(c.llm.enabled);
        assert_eq!(c.llm.api_key.expose(), "sk-secret");
        assert_eq!(c.llm.model, "claude-haiku-5-5");
        let dbg = format!("{c:?}");
        assert!(!dbg.contains("sk-secret") && !dbg.contains("tmdb-secret"));
        let c = Config::from_lookup(|k| match k {
            "MOONRISE_LLM_PROVIDER" => Some("openai".into()),
            "MOONRISE_LLM_BASE_URL" => Some("http://127.0.0.1:8092/v1".into()),
            _ => None,
        });
        assert!(c.llm.enabled, "openai-style endpoints may need no key");
        assert_eq!(c.llm.style, "openai");
    }

    #[test]
    fn llm_presets() {
        let lookup =
            |pairs: &'static [(&'static str, &'static str)]| move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string());
        // A preset fills in the address, model, format and the key's variable.
        let c = Config::from_lookup(lookup(&[
            ("MOONRISE_LLM_PROVIDER", "openrouter"),
            ("OPENROUTER_API_KEY", "or-key"),
            ("ANTHROPIC_API_KEY", "an-key"),
        ]));
        assert_eq!(
            (c.llm.base_url.as_str(), c.llm.style.as_str(), c.llm.output.as_str()),
            ("https://openrouter.ai/api/v1", "openai", "auto")
        );
        assert_eq!(c.llm.api_key.expose(), "or-key", "never another provider's key");
        assert!(c.llm.enabled);
        // A cloud preset needs its key; at another address (a proxy) it may not.
        let c = Config::from_lookup(lookup(&[("MOONRISE_LLM_PROVIDER", "openai"), ("ANTHROPIC_API_KEY", "an-key")]));
        assert!(!c.llm.enabled && c.llm.api_key.expose().is_empty());
        let c = Config::from_lookup(lookup(&[
            ("MOONRISE_LLM_PROVIDER", "openai"),
            ("MOONRISE_LLM_BASE_URL", "http://gw:4000/v1"),
        ]));
        assert!(c.llm.enabled);
        // Local servers: a model is needed, except llama.cpp (it serves what it loaded).
        let c = Config::from_lookup(lookup(&[("MOONRISE_LLM_PROVIDER", "ollama")]));
        assert!(!c.llm.enabled);
        let c = Config::from_lookup(lookup(&[("MOONRISE_LLM_PROVIDER", "ollama"), ("MOONRISE_LLM_MODEL", "qwen3:8b")]));
        assert!(c.llm.enabled && c.llm.api_key.expose().is_empty());
        let c = Config::from_lookup(lookup(&[("MOONRISE_LLM_API_STYLE", "llamacpp")]));
        assert!(c.llm.enabled && c.llm.base_url == "http://localhost:8080/v1");
        // Custom needs an address; unknown names are custom.
        let c = Config::from_lookup(lookup(&[("MOONRISE_LLM_PROVIDER", "vllm"), ("MOONRISE_LLM_MODEL", "m")]));
        assert_eq!((c.llm.provider.as_str(), c.llm.enabled), ("custom", false));
        // ChatGPT: only once signed in.
        let env = Env::from_pairs([("MOONRISE_LLM_PROVIDER", "chatgpt"), ("MOONRISE_LLM_MODEL", "m")]);
        assert!(!Config::resolve(&env, &Stored::default()).llm.enabled);
        let store = Stored::from_pairs([("llm.chatgpt", serde_json::json!({"client_id": "c", "refresh_token": "r"}))]);
        let c = Config::resolve(&env, &store);
        assert!(c.llm.enabled && c.llm.style == "responses");
    }

    /// Keys and headers from the environment never follow a provider or address chosen in the
    /// settings; the ChatGPT plan is always at OpenAI.
    #[test]
    fn env_credentials_stay_with_their_provider() {
        let j = |v: &str| serde_json::json!(v);
        let catcher = "http://catcher:1/v1";
        // Anthropic's key from the environment, an address from the settings: not sent there.
        let env = Env::from_pairs([("ANTHROPIC_API_KEY", "env-ant-key")]);
        let c = Config::resolve(&env, &Stored::from_pairs([("llm.base_url", j(catcher))]));
        assert_eq!(c.llm.api_key.expose(), "");
        assert_eq!(Config::resolve(&env, &Stored::default()).llm.api_key.expose(), "env-ant-key");
        // A generic key: only for the environment's provider and address.
        let env = Env::from_pairs([
            ("MOONRISE_LLM_API_KEY", "env-key"),
            ("MOONRISE_LLM_EXTRA_HEADERS", "X-Org: org-secret-1"),
        ]);
        let c = Config::resolve(
            &env,
            &Stored::from_pairs([("llm.provider", j("custom")), ("llm.base_url", j(catcher)), ("llm.model", j("m"))]),
        );
        assert!(c.llm.api_key.expose().is_empty() && c.llm.extra_headers.is_empty());
        let c = Config::resolve(&env, &Stored::from_pairs([("llm.provider", j("openrouter"))]));
        assert!(
            c.llm.api_key.expose().is_empty() && !c.llm.enabled,
            "an Anthropic key isn't OpenRouter's"
        );
        let env = Env::from_pairs([
            ("MOONRISE_LLM_PROVIDER", "custom"),
            ("MOONRISE_LLM_BASE_URL", "http://gw:1/v1"),
            ("MOONRISE_LLM_API_KEY", "env-key"),
            ("MOONRISE_LLM_MODEL", "m"),
        ]);
        let c = Config::resolve(&env, &Stored::default());
        assert_eq!((c.llm.api_key.expose(), c.llm.enabled), ("env-key", true));
        // A preset's own variable belongs to that preset, at its address.
        let env = Env::from_pairs([("OPENROUTER_API_KEY", "env-or-key")]);
        assert_eq!(
            Config::resolve(&env, &Stored::from_pairs([("llm.provider", j("openrouter"))]))
                .llm
                .api_key
                .expose(),
            "env-or-key"
        );
        assert_eq!(
            Config::resolve(
                &env,
                &Stored::from_pairs([("llm.provider", j("openrouter")), ("llm.base_url", j(catcher))])
            )
            .llm
            .api_key
            .expose(),
            ""
        );
        // A key stored with its address is the settings' own (provider changes handle those).
        let vault = |base: &str| {
            crate::llm::keys::change(
                Vec::new(),
                &crate::llm::keys::Binding::new("custom", base).unwrap(),
                Some("stored-key".into()),
            )
            .1
            .unwrap()
        };
        let c = Config::resolve(
            &Env::default(),
            &Stored::from_pairs([
                ("llm.provider", j("custom")),
                ("llm.base_url", j(catcher)),
                (crate::llm::keys::VAULT_KEY, vault(catcher)),
            ]),
        );
        assert_eq!(c.llm.api_key.expose(), "stored-key");
        // Bound to another address: not this configuration's key.
        let c = Config::resolve(
            &Env::default(),
            &Stored::from_pairs([
                ("llm.provider", j("custom")),
                ("llm.base_url", j(catcher)),
                (crate::llm::keys::VAULT_KEY, vault("http://gw:1/v1")),
            ]),
        );
        assert_eq!((c.llm.api_key.expose(), c.llm.key_for.as_ref()), ("", None));
        // A key left as a setting from before binding isn't used until it is bound.
        let c = Config::resolve(
            &Env::default(),
            &Stored::from_pairs([
                ("llm.provider", j("custom")),
                ("llm.base_url", j(catcher)),
                ("llm.api_key", j("stored-key")),
            ]),
        );
        assert_eq!(c.llm.api_key.expose(), "");
        // ChatGPT: always OpenAI's address, whatever is set.
        let c = Config::resolve(
            &Env::from_pairs([("MOONRISE_LLM_BASE_URL", catcher)]),
            &Stored::from_pairs([("llm.provider", j("chatgpt")), ("llm.base_url", j(catcher))]),
        );
        assert_eq!(c.llm.base_url, crate::llm::chatgpt::API_BASE);
    }

    #[test]
    fn builtin_keys_come_last() {
        let mut e = Env::from_pairs([("MOONRISE_NAME", "x")]);
        e.builtin = Builtin {
            tmdb: Secret::new("built-tmdb"),
            opensubtitles: Secret::new("built-os"),
        };
        let c = Config::resolve(&e, &Stored::default());
        assert_eq!((c.tmdb_key.expose(), c.opensubtitles.api_key.expose()), ("built-tmdb", "built-os"));
        assert!(
            c.opensubtitles.usable() && !c.opensubtitles.has_account(),
            "a consumer key alone: anonymous"
        );
        let store = Stored::from_pairs([("tmdb.api_key", serde_json::json!("stored-tmdb"))]);
        assert_eq!(Config::resolve(&e, &store).tmdb_key.expose(), "stored-tmdb", "the user's key wins");
        let mut e2 = Env::from_pairs([("TMDB_API_KEY", "env-tmdb")]);
        e2.builtin = e.builtin.clone();
        assert_eq!(Config::resolve(&e2, &store).tmdb_key.expose(), "env-tmdb", "the environment wins");
        assert!(!format!("{e:?}").contains("built-"), "never shown");
        assert_eq!(Builtin::default().var("TMDB_API_KEY"), None);
    }

    #[test]
    fn env_file_values_are_literal() {
        let v = parse_env_file("# c\nA=x$y\"z=1 #\r\n\nB = q\n");
        assert_eq!(
            v,
            vec![("A".to_string(), "x$y\"z=1 #".to_string()), ("B".to_string(), " q".to_string())]
        );
        let mut e = Env::from_pairs([("OPENSUBTITLES_API_KEY", "k"), ("MOONRISE_NAME", "")]);
        e.add_file("OPENSUBTITLES_API_KEY=other\nOPENSUBTITLES_USERNAME=u\nTMDB_API_KEY=ignored\n");
        assert_eq!(e.get("OPENSUBTITLES_API_KEY"), Some("k"), "the variable wins over the file");
        assert_eq!(e.get("OPENSUBTITLES_USERNAME"), Some("u"));
        assert!(e.is_from_file("OPENSUBTITLES_USERNAME") && !e.is_from_file("OPENSUBTITLES_API_KEY"));
        assert_eq!(e.get("TMDB_API_KEY"), None, "the file holds OpenSubtitles credentials only");
        assert_eq!(e.get("MOONRISE_NAME"), None, "empty variables count as unset");
        let c = Config::resolve(&e, &Stored::default());
        assert_eq!(c.opensubtitles.api_key.expose(), "k");
        assert!(c.opensubtitles.usable() && !c.opensubtitles.has_account());
        assert!(!format!("{c:?}").contains("\"k\""));
        assert!(!format!("{e:?}").contains("\"u\""), "Env's Debug hides values");
    }
}
