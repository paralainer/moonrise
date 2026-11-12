-- Track memory (server 0.29.0): the audio and subtitle tracks each user picked last in a
-- file (a version), so playback resumes with them on every device, and the next episode of a
-- show starts with the same language and kind (`userdata/tracks.rs`). `audio` and `subtitle`:
-- JSON {index, language, title, forced, sdh} (any may be missing); `subtitle_off` 1: subtitles
-- were turned off. NULL `audio` / `subtitle` with `subtitle_off` 0: not said. Additive: older
-- servers ignore the table.
CREATE TABLE user_file_tracks (
    user_id      TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    file_id      TEXT NOT NULL,
    item_id      TEXT NOT NULL,
    series_id    TEXT,
    audio        TEXT,
    subtitle     TEXT,
    subtitle_off INTEGER NOT NULL DEFAULT 0,
    updated_at   INTEGER NOT NULL,   -- Unix seconds
    PRIMARY KEY (user_id, file_id)
);
CREATE INDEX user_file_tracks_item ON user_file_tracks(user_id, item_id, updated_at);
CREATE INDEX user_file_tracks_series ON user_file_tracks(user_id, series_id, updated_at);
