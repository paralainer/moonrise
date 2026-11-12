-- Mixed libraries (server 0.15): libraries have no type; a file is a movie or an episode by its
-- name and folder (library/parse.rs). `kind` stays as what a library was made as, for older apps,
-- and may now be 'mixed' (the default). No other table refers to this one; older servers ignore
-- the wider check.
CREATE TABLE libraries_new (
    id    TEXT PRIMARY KEY,
    name  TEXT NOT NULL,
    kind  TEXT NOT NULL DEFAULT 'mixed' CHECK (kind IN ('movies', 'shows', 'mixed')),
    roots TEXT NOT NULL,
    sort  INTEGER NOT NULL
);
INSERT INTO libraries_new(id, name, kind, roots, sort) SELECT id, name, kind, roots, sort FROM libraries;
DROP TABLE libraries;
ALTER TABLE libraries_new RENAME TO libraries;
