-- Open-source Phase 3: fallbacks for optional services.
-- A title that went to review keeps the TMDB candidates it was scored against (Fix Match offers
-- them); local metadata (NFO files, the file's own tags, artwork in the folders, a frame of the
-- video) is looked at once per change of the title (local_at).
ALTER TABLE items ADD COLUMN candidates TEXT;
ALTER TABLE items ADD COLUMN local_at INTEGER;
