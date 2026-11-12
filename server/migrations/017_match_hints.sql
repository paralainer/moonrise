-- Match hints: ids and numbers stored once for a file's path or a series folder, which
-- identification trusts over a search (`identify_state = 'bootstrap'`). They come from a one-time
-- import in an early version, kept until now in `jf_map` in that import's own format; this copies
-- what matching reads into a table of our own. Nothing new is added to it.
--
-- Additive: `jf_map` is left as it was, so an older server still runs on this database. It is
-- dropped by a later migration, in a later release.
CREATE TABLE match_hints (
    id          INTEGER PRIMARY KEY,
    kind        TEXT NOT NULL CHECK (kind IN ('movie', 'episode', 'series')),
    path        TEXT NOT NULL,            -- the file (movie, episode) or folder (series)
    file_id     TEXT,                     -- our file (follows renames); set by the pass from `path`
    tmdb_id     INTEGER,
    imdb_id     TEXT,
    season_no   INTEGER,
    episode_no  INTEGER,
    episode_end INTEGER,
    title       TEXT,                     -- an episode's title until TMDB's arrives
    added_at    INTEGER                   -- Unix seconds: the "added" order
);
CREATE INDEX match_hints_file ON match_hints(file_id);
CREATE INDEX match_hints_path ON match_hints(path);

-- A provider id as identification read it from the import's JSON: the first key (in byte order)
-- equal to the name ignoring case, a non-empty string other than "0"; Tmdb an integer (at most
-- 18 characters, so it fits).
INSERT INTO match_hints(kind, path, file_id, tmdb_id, imdb_id, season_no, episode_no, episode_end, title, added_at)
SELECT
    lower(j.kind),
    j.jf_path,
    CASE WHEN j.kind = 'Series' THEN NULL ELSE j.file_id END,
    (SELECT CASE WHEN p.type = 'text' AND p.value NOT IN ('', '0')
                      AND (CASE WHEN substr(p.value, 1, 1) IN ('+', '-') THEN substr(p.value, 2) ELSE p.value END) GLOB '[0-9]*'
                      AND (CASE WHEN substr(p.value, 1, 1) IN ('+', '-') THEN substr(p.value, 2) ELSE p.value END) NOT GLOB '*[^0-9]*'
                      AND length(p.value) <= 18
                 THEN CAST(p.value AS INTEGER) END
       FROM json_each(CASE WHEN json_valid(j.provider_ids) THEN j.provider_ids ELSE '{}' END) p
      WHERE json_type(CASE WHEN json_valid(j.provider_ids) THEN j.provider_ids ELSE '{}' END) = 'object'
        AND lower(p.key) = 'tmdb'
      ORDER BY p.key LIMIT 1),
    (SELECT CASE WHEN p.type = 'text' AND p.value NOT IN ('', '0') THEN p.value END
       FROM json_each(CASE WHEN json_valid(j.provider_ids) THEN j.provider_ids ELSE '{}' END) p
      WHERE json_type(CASE WHEN json_valid(j.provider_ids) THEN j.provider_ids ELSE '{}' END) = 'object'
        AND lower(p.key) = 'imdb'
      ORDER BY p.key LIMIT 1),
    CASE WHEN j.kind = 'Series' THEN NULL ELSE j.season_no END,
    CASE WHEN j.kind = 'Series' THEN NULL ELSE j.episode_no END,
    CASE WHEN j.kind = 'Series' THEN NULL ELSE j.episode_end END,
    CASE WHEN j.kind = 'Series' THEN NULL ELSE j.name END,
    j.date_created
FROM jf_map j
WHERE j.kind IN ('Movie', 'Episode', 'Series') AND j.jf_path IS NOT NULL
ORDER BY j.rowid;
