-- Versions (server 0.20): one title, several files (cuts, qualities, dubs). Each user's choice
-- of version per title, where they stopped in each version (a cut of another length keeps its
-- own resume point), and the edition an NFO next to a file names (`<edition>`). Older servers
-- ignore all of it.
CREATE TABLE user_versions (
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    item_id    TEXT NOT NULL,
    file_id    TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (user_id, item_id)
);
CREATE TABLE user_file_positions (
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    file_id    TEXT NOT NULL,
    position   REAL NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (user_id, file_id)
);
-- The file the title's resume point was reached in (NULL: unknown, before 0.20).
ALTER TABLE user_data ADD COLUMN position_file TEXT;
-- An NFO's <edition> (NULL: none, or not read), read once per file stamp (edition_stamp).
ALTER TABLE files ADD COLUMN edition TEXT;
ALTER TABLE files ADD COLUMN edition_stamp TEXT;
