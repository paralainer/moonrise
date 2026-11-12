-- Phase 2: the library. Files on the media shares, the titles they are (identified from the
-- path, Jellyfin's ids once, TMDB, and an LLM for the ambiguous ones), TMDB metadata and
-- artwork, search, and the static map of Jellyfin ids.

-- Configured libraries (MOONRISE_LIBRARIES), rewritten at start.
CREATE TABLE libraries (
    id    TEXT PRIMARY KEY,               -- stable: derived from the name
    name  TEXT NOT NULL,
    kind  TEXT NOT NULL CHECK (kind IN ('movies', 'shows')),
    roots TEXT NOT NULL,                  -- JSON array of paths
    sort  INTEGER NOT NULL
);

-- Video files under the library roots.
CREATE TABLE files (
    id            TEXT PRIMARY KEY,       -- ULID; kept across renames (oshash + size)
    library_id    TEXT NOT NULL,
    path          TEXT NOT NULL UNIQUE,
    size          INTEGER NOT NULL,
    mtime         INTEGER NOT NULL,       -- ns since the epoch (as files_probe)
    oshash        TEXT,                   -- OpenSubtitles hash of (size, mtime) below
    oshash_stamp  TEXT,                   -- "size|mtime" the hash was made for
    container     TEXT,
    duration      REAL,
    bitrate       INTEGER,
    -- pending: not probed yet; main: a title (movie or episode); extra: in an extras folder;
    -- sample; short: under 2 minutes; broken: can't be probed.
    role          TEXT NOT NULL DEFAULT 'pending',
    item_id       TEXT,                   -- the movie or episode it plays
    version_label TEXT,
    parsed        TEXT,                   -- JSON: what the path says (parse.rs)
    first_seen    INTEGER NOT NULL,       -- Unix seconds
    last_seen     INTEGER NOT NULL,
    missing_since INTEGER,
    stable_since  INTEGER NOT NULL,       -- when (size, mtime) were first seen as they are
    probed_stamp  TEXT,                   -- "size|mtime" last probed (or failed)
    probe_error   TEXT,
    identified_at INTEGER,
    manual_key    TEXT                    -- set by POST /v1/admin/items/{id}/identify; never overridden
);
CREATE INDEX files_item ON files(item_id);
CREATE INDEX files_hash ON files(oshash, size);

-- Titles: movies, series, seasons, episodes.
CREATE TABLE items (
    id               TEXT PRIMARY KEY,    -- ULID, never changes
    kind             TEXT NOT NULL CHECK (kind IN ('movie', 'series', 'season', 'episode')),
    library_id       TEXT NOT NULL,
    -- Identity: tmdb:movie:603, tmdb:tv:1398, tmdb:tv:1398:s4, tmdb:tv:1398:s4e3, or (unknown)
    -- path:<library>/<relative path> (series: the folder).
    key              TEXT NOT NULL UNIQUE,
    parent_id        TEXT,                -- episode → season → series
    series_id        TEXT,
    season_no        INTEGER,
    episode_no       INTEGER,
    episode_end      INTEGER,
    title            TEXT NOT NULL,
    sort_title       TEXT,
    original_title   TEXT,
    year             INTEGER,
    premiere_date    TEXT,
    end_date         TEXT,
    status           TEXT,
    overview         TEXT,
    tagline          TEXT,
    official_rating  TEXT,
    community_rating REAL,
    critic_rating    REAL,
    genres           TEXT,                -- JSON arrays
    studios          TEXT,
    provider_ids     TEXT,                -- JSON object {Tmdb, Imdb, Tvdb}
    runtime          REAL,                -- seconds
    images           TEXT,                -- JSON {poster, backdrop[], logo, still}
    identify_state   TEXT NOT NULL,       -- bootstrap | matched | llm | review | manual
    identify_source  TEXT,
    confidence       REAL,
    identify_note    TEXT,
    added_at         INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL,
    refreshed_at     INTEGER              -- TMDB metadata fetched
);
CREATE INDEX items_parent ON items(parent_id);
CREATE INDEX items_series ON items(series_id);
CREATE INDEX items_lib ON items(library_id, kind);

-- Ids that were merged into others (re-identification): answered with the new item.
CREATE TABLE item_redirects (
    old_id TEXT PRIMARY KEY,
    new_id TEXT NOT NULL
);

CREATE TABLE people (
    id      TEXT PRIMARY KEY,
    tmdb_id INTEGER UNIQUE,
    name    TEXT NOT NULL,
    image   TEXT
);

CREATE TABLE item_people (
    item_id   TEXT NOT NULL,
    person_id TEXT NOT NULL,
    kind      TEXT NOT NULL,              -- Actor | Director | Writer
    role      TEXT,
    ord       INTEGER NOT NULL,
    PRIMARY KEY (item_id, person_id, kind)
);

-- Artwork. The id is a hash of the source's immutable file path, so it is also the cache tag.
CREATE TABLE images (
    id         TEXT PRIMARY KEY,
    source     TEXT NOT NULL,             -- tmdb:/abc.jpg
    kind       TEXT NOT NULL,             -- poster | backdrop | logo | still | profile
    width      INTEGER,
    height     INTEGER,
    path       TEXT,                      -- master, relative to /data/images
    bytes      INTEGER,
    fetched_at INTEGER,
    error      TEXT
);

-- Jellyfin's ids, read once by `bootstrap-jellyfin`; never refreshed from Jellyfin.
CREATE TABLE jf_map (
    jf_id        TEXT PRIMARY KEY,
    kind         TEXT NOT NULL,           -- Movie | Series | Season | Episode
    jf_path      TEXT,                    -- the file (movies, episodes) or folder (series)
    name         TEXT,
    year         INTEGER,
    series_jf_id TEXT,
    season_no    INTEGER,
    episode_no   INTEGER,
    episode_end  INTEGER,
    provider_ids TEXT,                    -- JSON
    runtime      REAL,
    media_source TEXT,                    -- JSON: Jellyfin's first media source (streams, labels)
    stamp_size   INTEGER,                 -- the file as it was when that was read
    stamp_mtime  INTEGER,
    date_created INTEGER,                 -- Jellyfin's DateCreated (Unix seconds): "added" order
    file_id      TEXT,                    -- our file (follows renames)
    imported_at  INTEGER NOT NULL
);
CREATE INDEX jf_map_file ON jf_map(file_id);
CREATE INDEX jf_map_path ON jf_map(jf_path);

-- Subtitles Jellyfin's OpenSubtitles plugin keeps in its own data directory, copied once.
CREATE TABLE jf_subtitles (
    jf_path     TEXT PRIMARY KEY,         -- where Jellyfin keeps it
    jf_item     TEXT NOT NULL,
    media_path  TEXT NOT NULL,            -- the video it belongs to
    language    TEXT,
    codec       TEXT,
    title       TEXT,
    forced      INTEGER NOT NULL DEFAULT 0,
    file        TEXT,                     -- copy, relative to /data/subs
    error       TEXT,
    imported_at INTEGER NOT NULL
);
CREATE INDEX jf_subtitles_media ON jf_subtitles(media_path);

-- LLM identification verdicts, so a file never flips between scans.
CREATE TABLE llm_verdicts (
    key            TEXT PRIMARY KEY,      -- hash(oshash/path, prompt version, model, candidates)
    model          TEXT NOT NULL,
    prompt_version INTEGER NOT NULL,
    request        TEXT NOT NULL,
    verdict        TEXT,
    accepted       INTEGER NOT NULL,
    reason         TEXT,
    input_tokens   INTEGER,
    output_tokens  INTEGER,
    created_at     INTEGER NOT NULL
);

-- TMDB answers (the URL without the key).
CREATE TABLE tmdb_cache (
    url        TEXT PRIMARY KEY,
    status     INTEGER NOT NULL,
    body       TEXT,
    fetched_at INTEGER NOT NULL
);

-- Directory mtimes seen by the quick scan.
CREATE TABLE scan_state (
    path       TEXT PRIMARY KEY,
    mtime      INTEGER NOT NULL,
    scanned_at INTEGER NOT NULL
);

-- Calls to paid/limited services per day: tmdb, llm (with tokens).
CREATE TABLE usage (
    day           TEXT NOT NULL,
    service       TEXT NOT NULL,
    calls         INTEGER NOT NULL DEFAULT 0,
    input_tokens  INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (day, service)
);

CREATE VIRTUAL TABLE items_fts USING fts5(
    item_id UNINDEXED, title, original_title, series_title,
    tokenize = 'unicode61 remove_diacritics 2'
);
