-- Which ffprobe made a stored probe (`ffprobe -version`'s first line): probes by another one
-- (Phase 0 ran Debian's) are made again by the scanner in the background.
ALTER TABLE files_probe ADD COLUMN prober TEXT;
