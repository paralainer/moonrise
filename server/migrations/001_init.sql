-- Phase 0: server identity and the persistent probe cache.

-- Key/value facts about this server: server_id (a UUID made once), schema_version.
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- ffprobe output and the keyframe index of a media file, valid while the file's size and
-- mtime are unchanged. Keyed by path for now; the Phase 2 scanner links it to files(id).
CREATE TABLE files_probe (
    path      TEXT PRIMARY KEY,
    size      INTEGER NOT NULL,
    mtime     INTEGER NOT NULL,      -- nanoseconds since the Unix epoch
    probe     TEXT NOT NULL,         -- ffprobe -show_streams -show_format JSON
    keyframes BLOB,                  -- video keyframe times, f64 seconds little-endian; NULL: not read
    chapters  TEXT,                  -- ffprobe -show_chapters JSON array
    probed_at INTEGER NOT NULL       -- Unix seconds
);
