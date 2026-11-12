-- The community databases' segment times (`library/community.rs`, server 0.28.0), keyed
-- by the database and the query asked (`tvdb_id=…&season=…&episode=…`), so a title matched
-- again with the same ids reuses them. `segments`: JSON [{kind, start, end}] in seconds, `end`
-- null for "to the end of the file"; [] when the database has none. Additive: older servers
-- ignore the table.
-- Number 018: 017 is the match hints'; this one applies after it.
CREATE TABLE community_segments (
    provider   TEXT NOT NULL,      -- theintrodb, introdb
    lookup     TEXT NOT NULL,
    fetched_at INTEGER NOT NULL,   -- Unix seconds
    segments   TEXT NOT NULL,
    PRIMARY KEY (provider, lookup)
);
