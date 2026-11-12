-- Drops the tables of a one-time import in an early version (server 0.32.0). Nothing has
-- read or written `jf_map` since migration 017 copied what identification needs into
-- `match_hints` (0.27.2); `jf_subtitles` was never read. Their indexes go with them.
--
-- Not harmless to 0.27.1 and older, which read `jf_map`: a database migrated by this release can't
-- be used by them. 0.27.2 and later never touch either table and run on it as before.
DROP TABLE IF EXISTS jf_map;
DROP TABLE IF EXISTS jf_subtitles;
