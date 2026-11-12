-- Search's Recent row (server 0.30.0): the titles each user opened from Search, newest
-- first, on every device (`userdata/search_start.rs`). At most 20 per user are kept. Additive:
-- older servers ignore the table.
CREATE TABLE user_search_recent (
    user_id   TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    item_id   TEXT NOT NULL,
    opened_at INTEGER NOT NULL,   -- Unix seconds
    PRIMARY KEY (user_id, item_id)
);
CREATE INDEX user_search_recent_order ON user_search_recent(user_id, opened_at);
