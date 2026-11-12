-- Ratings beside TMDB's (`library/ratings.rs`), keyed by IMDb id so a title matched again
-- picks up its new ratings. Additive: older servers ignore both tables.
CREATE TABLE ratings (
    imdb_id    TEXT NOT NULL,
    source     TEXT NOT NULL,      -- imdb, rotten_tomatoes, metacritic
    value      REAL NOT NULL,      -- imdb 0-10; rotten_tomatoes and metacritic 0-100
    votes      INTEGER,
    origin     TEXT NOT NULL,      -- imdb_dataset, omdb
    updated_at INTEGER NOT NULL,   -- Unix seconds
    PRIMARY KEY (imdb_id, source)
);

-- When a title was last looked up with a provider (OMDb), and whether it was found.
CREATE TABLE ratings_fetch (
    imdb_id    TEXT NOT NULL,
    provider   TEXT NOT NULL,
    fetched_at INTEGER NOT NULL,
    ok         INTEGER NOT NULL,
    PRIMARY KEY (imdb_id, provider)
);
