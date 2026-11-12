-- Phase 7: a proper Library. TMDB collections, and what the filters need that a scan knows
-- (the title's best file: resolution, HDR, length), kept beside the items.

-- TMDB collections ("Harry Potter Collection"), from the movies' `belongs_to_collection`.
CREATE TABLE collections (
    id         INTEGER PRIMARY KEY,       -- TMDB collection id
    name       TEXT NOT NULL,
    poster     TEXT,                      -- image ids (images table)
    backdrop   TEXT,
    updated_at INTEGER NOT NULL
);

-- A movie's collection (NULL: none, or not read yet) and when TMDB's answer was read for it
-- (NULL: not yet; the background backfill reads the movies that have none).
ALTER TABLE items ADD COLUMN collection_id INTEGER;
ALTER TABLE items ADD COLUMN collection_at INTEGER;
CREATE INDEX items_collection ON items(collection_id);

-- The video of a movie's or episode's best file (the largest, as playback picks), from its
-- stored probe. Refreshed after scan passes and probing; `stamp` says which file it was read from.
CREATE TABLE item_video (
    item_id  TEXT PRIMARY KEY,
    file_id  TEXT NOT NULL,
    stamp    TEXT NOT NULL,               -- "path|size|mtime"
    width    INTEGER,
    height   INTEGER,
    res      TEXT,                        -- 4k | 1080 | 720 | sd (NULL: no video stream)
    hdr      INTEGER NOT NULL DEFAULT 0,  -- HDR10, HLG or Dolby Vision
    range    TEXT,                        -- SDR, HDR10, HLG, DOVI…
    duration REAL                         -- the file's length (seconds)
);
CREATE INDEX item_video_res ON item_video(res);
