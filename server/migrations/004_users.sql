-- Phase 3: Moonrise's own users, tokens, device-code sign-in, watch state and playback sessions.
-- Independent of Jellyfin: nothing here is read from or written to Jellyfin.

CREATE TABLE users (
    id            TEXT PRIMARY KEY,       -- ULID
    name          TEXT NOT NULL UNIQUE COLLATE NOCASE,
    password_hash TEXT NOT NULL,          -- argon2id PHC string
    is_admin      INTEGER NOT NULL DEFAULT 0,
    created_at    INTEGER NOT NULL
);

-- Access tokens: 256-bit random, only their SHA-256 is kept. They don't expire; revocable.
CREATE TABLE tokens (
    hash        TEXT PRIMARY KEY,         -- hex SHA-256 of the token
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    device_id   TEXT,
    device_name TEXT,
    created_at  INTEGER NOT NULL,
    last_used   INTEGER,
    revoked_at  INTEGER
);
CREATE INDEX tokens_user ON tokens(user_id);

-- Device-code sign-in (the Quick Connect replacement): a TV shows user_code, someone signed
-- in approves it, the TV's poll then gets a token.
CREATE TABLE device_codes (
    device_code_hash TEXT PRIMARY KEY,    -- hex SHA-256 of the device code
    user_code        TEXT NOT NULL UNIQUE,
    device_id        TEXT,
    device_name      TEXT,
    created_at       INTEGER NOT NULL,
    expires_at       INTEGER NOT NULL,
    approved_by      TEXT REFERENCES users(id) ON DELETE CASCADE,
    approved_at      INTEGER,
    last_poll        INTEGER
);

-- Watch state per user and movie/episode (series and seasons are derived from their episodes).
CREATE TABLE user_data (
    user_id        TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    item_id        TEXT NOT NULL,
    position       REAL NOT NULL DEFAULT 0,   -- resume point, seconds (0: none)
    played         INTEGER NOT NULL DEFAULT 0,
    play_count     INTEGER NOT NULL DEFAULT 0,
    last_played_at INTEGER,                   -- Unix seconds
    updated_at     INTEGER NOT NULL,
    PRIMARY KEY (user_id, item_id)
);
CREATE INDEX user_data_recent ON user_data(user_id, last_played_at);

-- Playback sessions opened with a Moonrise token (the stream session id), so reports still land
-- after a restart, and a viewing counts once.
CREATE TABLE play_sessions (
    id             TEXT PRIMARY KEY,      -- the stream session id (sid)
    user_id        TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    item_id        TEXT NOT NULL,         -- Moonrise item id
    file_id        TEXT,
    duration       REAL,
    method         TEXT,
    started_at     INTEGER NOT NULL,
    last_report_at INTEGER,
    position       REAL,
    counted        INTEGER NOT NULL DEFAULT 0   -- this viewing has marked the item played
);
CREATE INDEX play_sessions_started ON play_sessions(started_at);
