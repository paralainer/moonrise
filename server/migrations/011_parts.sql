-- Multi-part episodes (server 0.17): one TMDB episode split over several files is one item per
-- file, Part 1, Part 2… (part 1 keeps the episode's key, part N gets `<key>:p<N>`). NULL: not a
-- part. Older servers ignore the column.
ALTER TABLE items ADD COLUMN part_no INTEGER;
