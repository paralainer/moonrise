//! TMDB metadata for identified items, in the usual settings (language `en`, certification
//! country `US` by default; both are settings): titles, overview, tagline, genres, studios, rating, dates, runtime, cast and
//! crew (the first 24 actors, directors and writers), external ids, artwork; for shows the
//! status, seasons and episodes (with each episode's TVDB/IMDb ids, which the app uses for
//! skip markers).
//!
//! Refresh: items added in the last 30 days weekly, continuing series daily, the rest monthly.

use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

use super::{
    images,
    tmdb::DAY,
    util::{now, sort_title},
    Library,
};

const MAX_PEOPLE: usize = 24;

/// The refresh interval for an item (seconds).
pub fn refresh_after(added_at: i64, status: Option<&str>, now: i64) -> i64 {
    if now - added_at < 30 * DAY {
        7 * DAY
    } else if matches!(status, Some("Returning Series" | "In Production" | "Planned")) {
        DAY
    } else {
        30 * DAY
    }
}

/// The names of a TMDB list (genres, companies, networks), each once: TMDB can list one twice
/// (two network entries named "Paramount+"), which counted a title twice in the facets.
fn names(v: &Value, key: &str) -> Value {
    let mut out: Vec<&str> = Vec::new();
    for n in v[key]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x["name"].as_str()).collect::<Vec<_>>())
        .unwrap_or_default()
    {
        if !out.contains(&n) {
            out.push(n);
        }
    }
    json!(out)
}

#[allow(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "TMDB's paths are lower case; matching .SVG too would change the pick"
)]
fn pick_image<'a>(list: &'a Value, lang: &str, prefer_null: bool) -> Option<&'a str> {
    let arr = list.as_array()?;
    let by_lang = |l: Option<&str>| {
        arr.iter()
            .find(|i| i["iso_639_1"].as_str() == l && i["file_path"].as_str().is_some_and(|f| !f.ends_with(".svg")))
    };
    let first = if prefer_null {
        by_lang(None).or_else(|| by_lang(Some(lang)))
    } else {
        by_lang(Some(lang)).or_else(|| by_lang(None))
    };
    first.and_then(|i| i["file_path"].as_str())
}

/// Artwork of a movie or show: poster, backdrops (the main one first), logo.
fn artwork(c: &rusqlite::Connection, d: &Value, lang: &str) -> Result<Value> {
    let poster = images::register(
        c,
        pick_image(&d["images"]["posters"], lang, false).or(d["poster_path"].as_str()),
        "poster",
    )?;
    let mut backdrops = Vec::new();
    let main = d["backdrop_path"].as_str();
    for fp in main.into_iter().chain(
        d["images"]["backdrops"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|b| b["iso_639_1"].is_null())
            .filter_map(|b| b["file_path"].as_str()),
    ) {
        if backdrops.len() >= 4 {
            break;
        }
        if let Some(id) = images::register(c, Some(fp), "backdrop")? {
            if !backdrops.contains(&id) {
                backdrops.push(id);
            }
        }
    }
    let logo = images::register(c, pick_image(&d["images"]["logos"], lang, false), "logo")?;
    Ok(json!({"poster": poster, "backdrop": backdrops, "logo": logo}))
}

fn certification_movie(d: &Value, country: &str) -> Option<String> {
    let find = |cc: &str| {
        d["release_dates"]["results"].as_array()?.iter().find(|r| r["iso_3166_1"] == cc)?["release_dates"]
            .as_array()?
            .iter()
            .filter_map(|x| x["certification"].as_str())
            .find(|c| !c.is_empty())
            .map(String::from)
    };
    find(country).or_else(|| find("US"))
}

fn certification_tv(d: &Value, country: &str) -> Option<String> {
    let find = |cc: &str| {
        d["content_ratings"]["results"].as_array()?.iter().find(|r| r["iso_3166_1"] == cc)?["rating"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    find(country).or_else(|| find("US"))
}

/// (person tmdb id, name, profile path, kind, role) in the usual order: actors first.
fn people(credits: &Value, tv: bool) -> Vec<(i64, String, Option<String>, &'static str, Option<String>)> {
    let mut out = Vec::new();
    for c in credits["cast"].as_array().into_iter().flatten() {
        let role = if tv {
            c["roles"][0]["character"].as_str()
        } else {
            c["character"].as_str()
        };
        if let (Some(id), Some(name)) = (c["id"].as_i64(), c["name"].as_str()) {
            out.push((
                id,
                name.to_string(),
                c["profile_path"].as_str().map(String::from),
                "Actor",
                role.map(String::from),
            ));
        }
    }
    let mut crew = Vec::new();
    for c in credits["crew"].as_array().into_iter().flatten() {
        let jobs: Vec<String> = if tv {
            c["jobs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|j| j["job"].as_str().map(String::from))
                .collect()
        } else {
            c["job"].as_str().map(String::from).into_iter().collect()
        };
        for job in jobs {
            let kind = match job.as_str() {
                "Director" => "Director",
                "Writer" | "Screenplay" | "Story" | "Novel" | "Teleplay" | "Characters" => "Writer",
                _ => continue,
            };
            if let (Some(id), Some(name)) = (c["id"].as_i64(), c["name"].as_str()) {
                if !crew
                    .iter()
                    .any(|x: &(i64, String, Option<String>, &str, Option<String>)| x.0 == id && x.3 == kind)
                {
                    crew.push((id, name.to_string(), c["profile_path"].as_str().map(String::from), kind, None));
                }
            }
        }
    }
    // Directors and writers always make the cut (directors first: TMDB lists a title's many
    // credited writers before them); actors fill the rest.
    crew.sort_by_key(|x| x.3 != "Director");
    let room = MAX_PEOPLE.saturating_sub(crew.len().min(8));
    out.truncate(room);
    crew.truncate(8);
    out.extend(crew);
    out
}

fn store_people(c: &rusqlite::Connection, item: &str, list: &[(i64, String, Option<String>, &str, Option<String>)]) -> Result<()> {
    c.execute("DELETE FROM item_people WHERE item_id = ?1", [item])?;
    for (ord, (tid, name, profile, kind, role)) in list.iter().enumerate() {
        let image = images::register(c, profile.as_deref(), "profile")?;
        let pid = tid.to_string();
        c.execute(
            "INSERT INTO people(id, tmdb_id, name, image) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, image = COALESCE(excluded.image, people.image)",
            params![pid, tid, name, image],
        )?;
        c.execute(
            "INSERT OR IGNORE INTO item_people(item_id, person_id, kind, role, ord) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![item, pid, kind, role, ord as i64],
        )?;
    }
    Ok(())
}

/// Merges provider ids: the bootstrap ones stay, TMDB's fill the gaps.
fn merge_ids(old: Option<&str>, new: &[(&str, Option<String>)]) -> String {
    let mut m: serde_json::Map<String, Value> = old.and_then(|o| serde_json::from_str(o).ok()).unwrap_or_default();
    for (k, v) in new {
        if let Some(v) = v.as_ref().filter(|v| !v.is_empty() && *v != "0") {
            if m.get(*k).and_then(|x| x.as_str()).is_none_or(str::is_empty) {
                m.insert(k.to_string(), json!(v));
            }
        }
    }
    Value::Object(m).to_string()
}

impl Library {
    /// Fetches metadata for identified items that are due; returns how many were updated.
    pub async fn refresh_metadata(&self, force: bool) -> Result<usize> {
        let Some(tmdb) = self.tmdb() else { return Ok(0) };
        let now = now();
        let due: Vec<(String, String, String, i64, Option<String>, Option<i64>)> = self
            .db
            .call(move |c| {
                let mut st = c.prepare("SELECT id, kind, key, added_at, status, refreshed_at FROM items WHERE kind IN ('movie', 'series') AND key LIKE 'tmdb:%'")?;
                let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?.collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await?;
        let due: Vec<_> = due
            .into_iter()
            .filter(|r| force || r.5.is_none_or(|at| now - at >= refresh_after(r.3, r.4.as_deref(), now)))
            .collect();
        // Seasons and episodes added since the series was refreshed need it too.
        let stale_series: Vec<String> = self
            .db
            .call(|c| {
                let mut st = c.prepare("SELECT DISTINCT series_id FROM items WHERE kind IN ('season', 'episode') AND refreshed_at IS NULL AND key LIKE 'tmdb:%'")?;
                let rows = st.query_map([], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
                Ok(rows)
            })
            .await?;
        let mut todo = due;
        if !stale_series.is_empty() {
            let ids = stale_series.clone();
            let more: Vec<_> = self
                .db
                .call(move |c| {
                    let mut out = Vec::new();
                    for id in ids {
                        if let Some(r) = c
                            .query_row(
                                "SELECT id, kind, key, added_at, status, refreshed_at FROM items WHERE id = ?1",
                                [id],
                                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
                            )
                            .optional()?
                        {
                            out.push(r);
                        }
                    }
                    Ok(out)
                })
                .await?;
            for m in more {
                if !todo.iter().any(|t| t.0 == m.0) {
                    todo.push(m);
                }
            }
        }
        if todo.is_empty() {
            return Ok(0);
        }
        tracing::info!(items = todo.len(), "fetching metadata");
        let mut n = 0;
        for (id, kind, key, added, status, refreshed) in todo {
            let Some(tid) = key.rsplit(':').next().and_then(|s| s.parse::<i64>().ok()) else {
                continue;
            };
            // Max age for the cached answer: what the refresh policy allows.
            let max_age = if refreshed.is_none() {
                30 * DAY
            } else {
                refresh_after(added, status.as_deref(), now)
            };
            let r = if kind == "movie" {
                self.apply_movie(&tmdb, &id, tid, max_age).await
            } else {
                self.apply_series(&tmdb, &id, tid, max_age).await
            };
            match r {
                Ok(()) => n += 1,
                Err(e) => tracing::warn!(item = id, key, "metadata: {e:#}"),
            }
        }
        Ok(n)
    }

    async fn apply_movie(&self, tmdb: &super::tmdb::Tmdb, id: &str, tid: i64, max_age: i64) -> Result<()> {
        let d = tmdb.movie(tid, max_age).await?;
        if d.is_null() {
            anyhow::bail!("TMDB has no movie {tid}");
        }
        let (id, lang, country) = (id.to_string(), tmdb.language.clone(), tmdb.country.clone());
        self.db
            .call(move |c| {
                let tx = c.transaction()?;
                let title = d["title"].as_str().unwrap_or("").to_string();
                let release = d["release_date"].as_str().filter(|s| !s.is_empty()).map(String::from);
                let old_ids: Option<String> = tx.query_row("SELECT provider_ids FROM items WHERE id = ?1", [&id], |r| r.get(0)).optional()?.flatten();
                let ids = merge_ids(
                    old_ids.as_deref(),
                    &[
                        ("Tmdb", Some(tid.to_string())),
                        ("Imdb", d["external_ids"]["imdb_id"].as_str().or(d["imdb_id"].as_str()).map(String::from)),
                        ("TmdbCollection", d["belongs_to_collection"]["id"].as_i64().map(|i| i.to_string())),
                    ],
                );
                let art = artwork(&tx, &d, &lang)?;
                tx.execute(
                    "UPDATE items SET title = ?2, sort_title = ?3, original_title = ?4, year = ?5, premiere_date = ?6, overview = ?7, tagline = ?8,
                     official_rating = ?9, community_rating = ?10, genres = ?11, studios = ?12, provider_ids = ?13, runtime = ?14, images = ?15,
                     status = ?16, refreshed_at = ?17, updated_at = ?17 WHERE id = ?1",
                    params![
                        id,
                        title,
                        sort_title(&title),
                        d["original_title"].as_str(),
                        release.as_deref().and_then(|r| r.get(..4)).and_then(|y| y.parse::<i32>().ok()),
                        release,
                        d["overview"].as_str().filter(|s| !s.is_empty()),
                        d["tagline"].as_str().filter(|s| !s.is_empty()),
                        certification_movie(&d, &country),
                        d["vote_average"].as_f64().filter(|v| *v > 0.0).map(|v| (v * 10.0).round() / 10.0),
                        names(&d, "genres").to_string(),
                        names(&d, "production_companies").to_string(),
                        ids,
                        d["runtime"].as_i64().filter(|r| *r > 0).map(|r| r as f64 * 60.0),
                        art.to_string(),
                        d["status"].as_str(),
                        now(),
                    ],
                )?;
                store_people(&tx, &id, &people(&d["credits"], false))?;
                super::browse::store_collection(&tx, &id, &d)?;
                tx.commit()?;
                Ok(())
            })
            .await
    }

    async fn apply_series(&self, tmdb: &super::tmdb::Tmdb, id: &str, tid: i64, max_age: i64) -> Result<()> {
        let d = tmdb.tv(tid, max_age).await?;
        if d.is_null() {
            anyhow::bail!("TMDB has no show {tid}");
        }
        // Seasons and episodes we have.
        let sid = id.to_string();
        let seasons: Vec<(String, i64)> = self
            .db
            .call(move |c| {
                let mut st = c.prepare("SELECT id, season_no FROM items WHERE kind = 'season' AND parent_id = ?1")?;
                let rows = st
                    .query_map([sid], |r| Ok((r.get(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0))))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await?;
        let mut season_data = Vec::new();
        for (season_id, no) in &seasons {
            let sv = tmdb.season(tid, *no, max_age).await.unwrap_or(Value::Null);
            season_data.push((season_id.clone(), *no, sv));
        }
        // Episode ids (TVDB/IMDb) for episodes that have none yet.
        let sid = id.to_string();
        let episodes: Vec<(String, i64, i64, Option<String>)> = self
            .db
            .call(move |c| {
                // (Parts after the first get no TVDB/IMDb episode ids: those are the whole episode's, and the
                // app would look up its skip markers by them.)
                let mut st = c.prepare("SELECT id, season_no, episode_no, provider_ids FROM items WHERE kind = 'episode' AND series_id = ?1 AND episode_no IS NOT NULL AND COALESCE(part_no, 1) = 1")?;
                let rows = st.query_map([sid], |r| Ok((r.get(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0), r.get(2)?, r.get(3)?)))?.collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await?;
        let mut ep_ids: Vec<(String, Value)> = Vec::new();
        for (eid, s, e, ids) in &episodes {
            let has_tvdb = ids.as_deref().is_some_and(|i| i.contains("\"Tvdb\""));
            // Long-lived: episode ids hardly change.
            let v = tmdb
                .episode_ids(tid, *s, *e, if has_tvdb { 365 * DAY } else { max_age.max(30 * DAY) })
                .await
                .unwrap_or(Value::Null);
            ep_ids.push((eid.clone(), v));
        }
        let (id, lang, country) = (id.to_string(), tmdb.language.clone(), tmdb.country.clone());
        self.db
            .call(move |c| {
                let tx = c.transaction()?;
                let title = d["name"].as_str().unwrap_or("").to_string();
                let first = d["first_air_date"].as_str().filter(|s| !s.is_empty()).map(String::from);
                let status = d["status"].as_str().map(String::from);
                let end = if matches!(status.as_deref(), Some("Ended" | "Canceled")) { d["last_air_date"].as_str().map(String::from) } else { None };
                let old_ids: Option<String> = tx.query_row("SELECT provider_ids FROM items WHERE id = ?1", [&id], |r| r.get(0)).optional()?.flatten();
                let ids = merge_ids(
                    old_ids.as_deref(),
                    &[
                        ("Tmdb", Some(tid.to_string())),
                        ("Imdb", d["external_ids"]["imdb_id"].as_str().map(String::from)),
                        ("Tvdb", d["external_ids"]["tvdb_id"].as_i64().map(|i| i.to_string())),
                    ],
                );
                let art = artwork(&tx, &d, &lang)?;
                let runtime = d["episode_run_time"].as_array().and_then(|a| a.first()).and_then(serde_json::Value::as_i64).map(|r| r as f64 * 60.0);
                tx.execute(
                    "UPDATE items SET title = ?2, sort_title = ?3, original_title = ?4, year = ?5, premiere_date = ?6, end_date = ?7, status = ?8,
                     overview = ?9, tagline = ?10, official_rating = ?11, community_rating = ?12, genres = ?13, studios = ?14, provider_ids = ?15,
                     runtime = ?16, images = ?17, refreshed_at = ?18, updated_at = ?18 WHERE id = ?1",
                    params![
                        id,
                        title,
                        sort_title(&title),
                        d["original_name"].as_str(),
                        first.as_deref().and_then(|r| r.get(..4)).and_then(|y| y.parse::<i32>().ok()),
                        first,
                        end,
                        status,
                        d["overview"].as_str().filter(|s| !s.is_empty()),
                        d["tagline"].as_str().filter(|s| !s.is_empty()),
                        certification_tv(&d, &country),
                        d["vote_average"].as_f64().filter(|v| *v > 0.0).map(|v| (v * 10.0).round() / 10.0),
                        names(&d, "genres").to_string(),
                        names(&d, "networks").to_string(),
                        ids,
                        runtime,
                        art.to_string(),
                        now(),
                    ],
                )?;
                store_people(&tx, &id, &people(&d["aggregate_credits"], true))?;
                for (season_id, no, sv) in &season_data {
                    let name = sv["name"].as_str().map_or_else(|| if *no == 0 { "Specials".into() } else { format!("Season {no}") }, String::from);
                    let poster = images::register(&tx, sv["poster_path"].as_str(), "poster")?;
                    let air = sv["air_date"].as_str().filter(|s| !s.is_empty());
                    tx.execute(
                        "UPDATE items SET title = ?2, sort_title = ?2, overview = ?3, premiere_date = ?4, year = ?5, images = ?6, refreshed_at = ?7, updated_at = ?7 WHERE id = ?1",
                        params![
                            season_id,
                            name,
                            sv["overview"].as_str().filter(|s| !s.is_empty()),
                            air,
                            air.and_then(|r| r.get(..4)).and_then(|y| y.parse::<i32>().ok()),
                            json!({"poster": poster, "backdrop": [], "logo": null}).to_string(),
                            now()
                        ],
                    )?;
                    for e in sv["episodes"].as_array().into_iter().flatten() {
                        let Some(en) = e["episode_number"].as_i64() else { continue };
                        let still = images::register(&tx, e["still_path"].as_str(), "still")?;
                        let air = e["air_date"].as_str().filter(|s| !s.is_empty());
                        tx.execute(
                            // (A part of a split episode keeps its own runtime: its file's.)
                            "UPDATE items SET title = ?3, sort_title = ?3, overview = ?4, premiere_date = ?5, year = ?6, community_rating = ?7,
                             runtime = CASE WHEN part_no IS NULL THEN ?8 ELSE runtime END,
                             images = ?9, refreshed_at = ?10, updated_at = ?10 WHERE kind = 'episode' AND parent_id = ?1 AND episode_no = ?2",
                            params![
                                season_id,
                                en,
                                e["name"].as_str().unwrap_or(""),
                                e["overview"].as_str().filter(|s| !s.is_empty()),
                                air,
                                air.and_then(|r| r.get(..4)).and_then(|y| y.parse::<i32>().ok()),
                                e["vote_average"].as_f64().filter(|v| *v > 0.0).map(|v| (v * 10.0).round() / 10.0),
                                e["runtime"].as_i64().filter(|r| *r > 0).map(|r| r as f64 * 60.0),
                                json!({"still": still}).to_string(),
                                now()
                            ],
                        )?;
                    }
                }
                for (eid, v) in &ep_ids {
                    let old: Option<String> = tx.query_row("SELECT provider_ids FROM items WHERE id = ?1", [eid], |r| r.get(0)).optional()?.flatten();
                    let ids = merge_ids(
                        old.as_deref(),
                        &[
                            ("Tvdb", v["tvdb_id"].as_i64().map(|i| i.to_string())),
                            ("Imdb", v["imdb_id"].as_str().map(String::from)),
                            ("Tmdb", v["id"].as_i64().map(|i| i.to_string())),
                        ],
                    );
                    tx.execute("UPDATE items SET provider_ids = ?2 WHERE id = ?1", params![eid, ids])?;
                }
                // Episodes TMDB doesn't list keep their parsed title; mark them refreshed too.
                tx.execute("UPDATE items SET refreshed_at = ?2 WHERE series_id = ?1 AND refreshed_at IS NULL", params![id, now()])?;
                tx.commit()?;
                Ok(())
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_once_each() {
        let v = json!({"networks": [{"name": "Paramount+"}, {"name": "CBS"}, {"name": "Paramount+"}], "genres": []});
        assert_eq!(names(&v, "networks"), json!(["Paramount+", "CBS"]));
        assert_eq!(names(&v, "genres"), json!([]));
        assert_eq!(names(&v, "missing"), json!([]));
    }

    #[test]
    fn refresh_policy() {
        let now = 100 * DAY;
        assert_eq!(refresh_after(now - DAY, None, now), 7 * DAY);
        assert_eq!(refresh_after(0, Some("Returning Series"), now), DAY);
        assert_eq!(refresh_after(0, Some("Ended"), now), 30 * DAY);
    }

    #[test]
    fn ids_merge_keeps_bootstrap() {
        let m = merge_ids(
            Some(r#"{"Tvdb":"1","Imdb":"tt1"}"#),
            &[("Tvdb", Some("2".into())), ("Tmdb", Some("5".into())), ("Imdb", None)],
        );
        let v: Value = serde_json::from_str(&m).unwrap();
        assert_eq!(v, json!({"Tvdb": "1", "Imdb": "tt1", "Tmdb": "5"}));
    }

    #[test]
    fn credits() {
        let credits = json!({"cast": (0..30).map(|i| json!({"id": i, "name": format!("A{i}"), "character": "x"})).collect::<Vec<_>>(),
                             "crew": [{"id": 100, "name": "D", "job": "Director"}, {"id": 101, "name": "W", "job": "Screenplay"}, {"id": 102, "name": "P", "job": "Producer"}]});
        let p = people(&credits, false);
        assert_eq!(p.len(), 24);
        assert!(p.iter().any(|x| x.3 == "Director") && p.iter().any(|x| x.3 == "Writer"));
        // Many writers listed first don't push the director out.
        let mut crew: Vec<Value> = (0..12)
            .map(|i| json!({"id": 200 + i, "name": format!("W{i}"), "job": "Writer"}))
            .collect();
        crew.push(json!({"id": 300, "name": "Dir", "job": "Director"}));
        let p = people(&json!({"cast": [], "crew": crew}), false);
        assert_eq!(p.iter().filter(|x| x.3 == "Director").count(), 1);
        assert!(!p.iter().any(|x| x.1 == "P"));
    }
}
