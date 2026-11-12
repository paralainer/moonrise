-- Continue Watching: a thread (a show, a TMDB collection "collection:<id>", or a film)
-- removed from the row by a user, on any device. It stays out until there is activity in it
-- after `hidden_at`. Additive: older servers ignore the table.
CREATE TABLE continue_hidden (
    user_id   TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    thread    TEXT NOT NULL,
    hidden_at INTEGER NOT NULL,  -- Unix seconds
    PRIMARY KEY (user_id, thread)
);
