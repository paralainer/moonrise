-- Where an episode's end credits start, found in the file itself (`credits.rs`, server 0.23.2):
-- JSON {"v": version, "start": seconds or null}, made once per file when it is first played.
-- Older servers don't read it, and `probe_put` of theirs drops it (it is made again then).
ALTER TABLE files_probe ADD COLUMN credits TEXT;
