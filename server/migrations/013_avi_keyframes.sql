-- AVI keyframe indexes (server 0.22.1) are the keyframes' shown times, not their decode times
-- (`scan::parse`): the stored ones are made again in the background (`index_backfill.rs`), or when
-- the title is next played.
UPDATE files_probe SET keyframes = NULL
WHERE keyframes IS NOT NULL AND json_extract(probe, '$.format.format_name') = 'avi';
