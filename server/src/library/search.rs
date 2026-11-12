//! Search: SQLite FTS5 over titles, original titles and series names (`unicode61
//! remove_diacritics 2`, every word a prefix), titles and episodes apart, like the app's two
//! queries.

use anyhow::Result;
use rusqlite::Connection;

/// Rebuilds the index from `items` (a few thousand rows: milliseconds).
pub fn rebuild(c: &mut Connection) -> Result<()> {
    let tx = c.transaction()?;
    tx.execute("DELETE FROM items_fts", [])?;
    tx.execute(
        "INSERT INTO items_fts(item_id, title, original_title, series_title)
         SELECT i.id, i.title, COALESCE(NULLIF(i.original_title, i.title), ''), COALESCE(s.title, '')
         FROM items i LEFT JOIN items s ON s.id = i.series_id WHERE i.kind IN ('movie', 'series', 'episode')",
        [],
    )?;
    tx.commit()?;
    Ok(())
}

/// The FTS query for what a user typed: each word a quoted prefix, all required. None when
/// there is nothing to search for.
pub fn fts_query(q: &str, columns: &str) -> Option<String> {
    let words: Vec<String> = q
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{}\"*", w.replace('"', "")))
        .collect();
    (!words.is_empty()).then(|| format!("{{{columns}}} : {}", words.join(" ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries() {
        assert_eq!(
            fts_query("sopra", "title original_title").unwrap(),
            "{title original_title} : \"sopra\"*"
        );
        assert_eq!(fts_query("Thor: love", "title").unwrap(), "{title} : \"Thor\"* \"love\"*");
        assert!(fts_query(" -- ", "title").is_none());
    }

    #[test]
    fn index_finds_prefixes_without_diacritics() {
        let mut c = Connection::open_in_memory().unwrap();
        c.execute_batch(include_str!("../../migrations/002_library.sql")).unwrap();
        c.execute_batch(
            "INSERT INTO items(id, kind, library_id, key, title, identify_state, added_at, updated_at) VALUES
             ('s', 'series', 'shows', 'k1', 'Shōgun', 'matched', 0, 0),
             ('e', 'episode', 'shows', 'k2', 'Anjin', 'matched', 0, 0),
             ('m', 'movie', 'movies', 'k3', 'The Sopranos Movie', 'matched', 0, 0);
             UPDATE items SET series_id = 's' WHERE id = 'e';",
        )
        .unwrap();
        rebuild(&mut c).unwrap();
        let find = |q: &str, cols: &str| -> Vec<String> {
            let mut st = c
                .prepare("SELECT item_id FROM items_fts WHERE items_fts MATCH ?1 ORDER BY rank")
                .unwrap();
            st.query_map([fts_query(q, cols).unwrap()], |r| r.get(0))
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
        };
        assert_eq!(find("shogun", "title original_title"), vec!["s"]);
        assert_eq!(find("sopr", "title original_title"), vec!["m"]);
        assert_eq!(find("shog anj", "title original_title series_title"), vec!["e"]);
    }
}
