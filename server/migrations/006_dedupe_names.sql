-- A title's genres and studios (networks) each once: TMDB can list one twice (two "Paramount+"
-- networks), and the Library's facets counted such a title twice. First spellings kept, in order.
UPDATE items SET studios = (SELECT json_group_array(value) FROM (SELECT value, MIN(key) AS k FROM json_each(items.studios) GROUP BY value ORDER BY k))
 WHERE json_valid(studios) AND (SELECT COUNT(*) FROM json_each(items.studios)) > (SELECT COUNT(DISTINCT value) FROM json_each(items.studios));
UPDATE items SET genres = (SELECT json_group_array(value) FROM (SELECT value, MIN(key) AS k FROM json_each(items.genres) GROUP BY value ORDER BY k))
 WHERE json_valid(genres) AND (SELECT COUNT(*) FROM json_each(items.genres)) > (SELECT COUNT(DISTINCT value) FROM json_each(items.genres));
