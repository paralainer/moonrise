//! Small helpers: ULIDs, title normalisation and similarity, sort titles.

use std::time::{SystemTime, UNIX_EPOCH};

use unicode_normalization::UnicodeNormalization;

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// A ULID in lower case (26 characters of Crockford base32: 48-bit milliseconds, 80 random
/// bits). Sortable and URL-safe.
pub fn ulid() -> String {
    const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let rnd: u128 = rand::random::<u128>() & ((1u128 << 80) - 1);
    let v = (ms << 80) | rnd;
    (0..26).rev().map(|i| ALPHABET[((v >> (i * 5)) & 31) as usize] as char).collect()
}

/// Cyrillic → Latin (roughly the transliteration release names use).
fn translit(c: char) -> Option<&'static str> {
    Some(match c {
        'а' => "a",
        'б' => "b",
        'в' => "v",
        'г' => "g",
        'д' => "d",
        'е' | 'ё' | 'э' | 'є' => "e",
        'ж' => "zh",
        'з' => "z",
        'и' | 'і' => "i",
        'й' | 'ы' => "y",
        'к' => "k",
        'л' => "l",
        'м' => "m",
        'н' => "n",
        'о' => "o",
        'п' => "p",
        'р' => "r",
        'с' => "s",
        'т' => "t",
        'у' => "u",
        'ф' => "f",
        'х' => "kh",
        'ц' => "ts",
        'ч' => "ch",
        'ш' => "sh",
        'щ' => "shch",
        'ъ' | 'ь' => "",
        'ю' => "yu",
        'я' => "ya",
        'ї' => "yi",
        _ => return None,
    })
}

/// Lower case, no diacritics, Cyrillic transliterated, "&" as "and", punctuation dropped,
/// single spaces. Roman numerals II-V stay as they are.
pub fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.nfkd() {
        if unicode_normalization::char::is_combining_mark(c) {
            continue;
        }
        for c in c.to_lowercase() {
            if let Some(t) = translit(c) {
                out.push_str(t);
            } else if c == '&' {
                out.push_str(" and ");
            } else if c.is_alphanumeric() {
                out.push(c);
            } else if c == '\'' || c == '’' {
                // "Devil's" = "Devils"
            } else {
                out.push(' ');
            }
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + usize::from(ca != cb));
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Title similarity in 0..1 after normalisation: edit distance, with credit for a title that
/// is the other's main part ("Dungeons and Dragons" ~ "Dungeons & Dragons: Honor Among
/// Thieves") and for the same words in another order.
pub fn similarity(a: &str, b: &str) -> f64 {
    let (na, nb) = (normalize(a), normalize(b));
    if na.is_empty() || nb.is_empty() {
        return 0.0;
    }
    if na == nb {
        return 1.0;
    }
    let (ca, cb): (Vec<char>, Vec<char>) = (na.chars().collect(), nb.chars().collect());
    let lev = 1.0 - levenshtein(&ca, &cb) as f64 / ca.len().max(cb.len()) as f64;
    // Without the subtitle after ":" or " - ".
    let main = |s: &str| normalize(s.split([':', '(']).next().unwrap_or(s).split(" - ").next().unwrap_or(s));
    let prefix = if main(a) == nb || main(b) == na || (!main(a).is_empty() && main(a) == main(b)) {
        0.9
    } else {
        0.0
    };
    let wa: std::collections::BTreeSet<&str> = na.split(' ').collect();
    let wb: std::collections::BTreeSet<&str> = nb.split(' ').collect();
    let jaccard = wa.intersection(&wb).count() as f64 / wa.union(&wb).count() as f64;
    lev.max(prefix).max(jaccard * 0.95)
}

/// Sort title: without a leading article.
pub fn sort_title(title: &str) -> String {
    let t = title.trim();
    for a in ["The ", "A ", "An "] {
        if t.len() > a.len() && t.get(..a.len()).is_some_and(|p| p.eq_ignore_ascii_case(a)) {
            return t[a.len()..].to_string();
        }
    }
    t.to_string()
}

/// The TMDB id of a TV key (`tmdb:tv:1398`, or an episode's/season's `tmdb:tv:1398:s4e3`).
pub fn tmdb_tv_id(key: &str) -> Option<i64> {
    key.strip_prefix("tmdb:tv:")?.split(':').next()?.parse().ok()
}

/// The TMDB id of a movie key (`tmdb:movie:603`).
pub fn tmdb_movie_id(key: &str) -> Option<i64> {
    key.strip_prefix("tmdb:movie:")?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ulids() {
        let a = ulid();
        let b = ulid();
        assert_eq!(a.len(), 26);
        assert_ne!(a, b);
    }

    #[test]
    fn normalising() {
        assert_eq!(normalize("Shōgun"), "shogun");
        assert_eq!(normalize("Dungeons & Dragons: Honor"), "dungeons and dragons honor");
        assert_eq!(normalize("The Devil's Advocate"), "the devils advocate");
        assert_eq!(normalize("Ван Хельсинг"), "van khelsing");
    }

    #[test]
    fn similar() {
        assert_eq!(similarity("Thor - The Dark World", "Thor: The Dark World"), 1.0);
        assert!(similarity("Dungeons And Dragons", "Dungeons & Dragons: Honor Among Thieves") >= 0.9);
        assert!(similarity("Puss in Boots", "Puss in Boots: The Last Wish") >= 0.9);
        assert!(similarity("Kong Skull Island", "Kong: Skull Island") > 0.99);
        assert!(similarity("Jurassic Park", "Zootopia") < 0.3);
    }

    #[test]
    fn sort_titles() {
        assert_eq!(sort_title("The Office"), "Office");
        assert_eq!(sort_title("A Bug's Life"), "Bug's Life");
        assert_eq!(sort_title("Thea"), "Thea");
        assert_eq!(sort_title("Don’t It Make"), "Don’t It Make");
    }
}

/// `now - interval`: when a rate limiter first acts, the last call seemingly an interval ago,
/// so the first call isn't delayed. A monotonic clock younger than `interval` (a host just booted)
/// can't go back that far: then `now`, and the first call waits at most `interval`.
pub fn interval_ago(now: std::time::Instant, interval: std::time::Duration) -> std::time::Instant {
    now.checked_sub(interval).unwrap_or(now)
}

#[cfg(test)]
mod interval_ago_tests {
    use std::time::{Duration, Instant};

    #[test]
    fn an_interval_back_when_the_clock_reaches() {
        let now = Instant::now();
        let d = Duration::from_millis(250);
        assert_eq!(Some(super::interval_ago(now, d)), now.checked_sub(d));
    }

    /// Further back than the clock goes: `now`, no panic (as `now - d` would).
    #[test]
    fn now_when_the_clock_is_younger_than_the_interval() {
        let now = Instant::now();
        assert!(now.checked_sub(Duration::MAX).is_none());
        assert_eq!(super::interval_ago(now, Duration::MAX), now);
    }
}

/// The redirect policy of an HTTP client that sends credentials in headers or the URL (TMDB,
/// OpenSubtitles): redirects within the same origin (scheme, host, port) are followed, any other
/// is not (the client answers the redirect itself, which callers treat as an error), so a key
/// is never carried to another server.
pub fn same_origin_redirects() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        let first = attempt.previous().first().map(reqwest::Url::origin);
        if attempt.previous().len() >= 10 || first.as_ref() != Some(&attempt.url().origin()) {
            attempt.stop()
        } else {
            attempt.follow()
        }
    })
}

#[cfg(test)]
mod redirect_tests {
    /// Same origin: followed; another server: the redirect is answered as it is.
    #[tokio::test]
    async fn only_same_origin_redirects_are_followed() {
        use axum::{response::Redirect, routing::get, Router};
        let other = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let other_addr = other.local_addr().unwrap();
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let h2 = hits.clone();
        tokio::spawn(async move {
            axum::serve(
                other,
                Router::new().fallback(move || {
                    h2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    async { "caught" }
                }),
            )
            .await
            .unwrap();
        });
        let to = format!("http://{other_addr}/x");
        let app = Router::new()
            .route("/here", get(|| async { Redirect::temporary("/there") }))
            .route("/there", get(|| async { "ok" }))
            .route(
                "/away",
                get(move || {
                    let to = to.clone();
                    async move { Redirect::permanent(&to) }
                }),
            );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let a = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        let c = reqwest::Client::builder().redirect(super::same_origin_redirects()).build().unwrap();
        let r = c.get(format!("http://{a}/here")).header("Api-Key", "k").send().await.unwrap();
        assert_eq!(r.text().await.unwrap(), "ok");
        let r = c.get(format!("http://{a}/away")).header("Api-Key", "k").send().await.unwrap();
        assert!(r.status().is_redirection());
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
}
