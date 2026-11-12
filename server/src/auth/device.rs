//! Device-code sign-in (like Quick Connect). A device without a keyboard (the
//! TV) asks for a code pair, shows the short `user_code` and polls with the long, secret
//! `device_code`; someone signed in approves the user code (the iPhone app, or the `/link` page
//! with a password); the next poll gets a token for that user. Codes live 10 minutes, are
//! used once, and only the device code's SHA-256 is stored.

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use super::{issue_token, sha256_hex, User};

pub const EXPIRES_SECS: i64 = 600;
pub const INTERVAL_SECS: i64 = 5;
/// No vowels (no words), no look-alikes (0/O, 1/I).
const LETTERS: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ";

pub struct Started {
    pub device_code: String,
    pub user_code: String,
}

#[derive(Debug, PartialEq)]
pub enum Poll {
    Pending,
    /// Polled faster than the interval.
    SlowDown,
    Expired,
    Approved {
        token: String,
        user: User,
    },
}

/// "abcd 1234", "ABCD1234" → "ABCD-1234".
pub fn normalize_user_code(s: &str) -> Option<String> {
    let c: String = s
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    (c.len() == 8 && c[..4].bytes().all(|b| b.is_ascii_uppercase()) && c[4..].bytes().all(|b| b.is_ascii_digit()))
        .then(|| format!("{}-{}", &c[..4], &c[4..]))
}

fn new_user_code() -> String {
    use rand::Rng;
    let mut r = rand::thread_rng();
    let letters: String = (0..4).map(|_| LETTERS[r.gen_range(0..LETTERS.len())] as char).collect();
    format!("{letters}-{:04}", r.gen_range(0..10_000))
}

pub fn start(c: &Connection, device_id: Option<&str>, device_name: Option<&str>, now: i64) -> Result<Started> {
    c.execute("DELETE FROM device_codes WHERE expires_at < ?1", [now - 3600])?;
    let device_code = super::new_token().replacen("moonrise_", "dc_", 1);
    for _ in 0..20 {
        let user_code = new_user_code();
        let n = c.execute(
            "INSERT OR IGNORE INTO device_codes(device_code_hash, user_code, device_id, device_name, created_at, expires_at)
             SELECT ?1, ?2, ?3, ?4, ?5, ?6 WHERE NOT EXISTS (SELECT 1 FROM device_codes WHERE user_code = ?2)",
            params![
                sha256_hex(&device_code),
                user_code,
                device_id.map(super::clip),
                device_name.map(super::clip),
                now,
                now + EXPIRES_SECS
            ],
        )?;
        if n == 1 {
            return Ok(Started { device_code, user_code });
        }
    }
    anyhow::bail!("no free user code")
}

/// Approves a pending, unexpired code for `user_id`. Returns the device's name, or None when
/// there is no such code (unknown, expired, or approved already).
pub fn approve(c: &Connection, user_code: &str, user_id: &str, now: i64) -> Result<Option<Option<String>>> {
    let Some(code) = normalize_user_code(user_code) else {
        return Ok(None);
    };
    let name: Option<Option<String>> = c
        .query_row(
            "SELECT device_name FROM device_codes WHERE user_code = ?1 AND expires_at >= ?2 AND approved_by IS NULL",
            params![code, now],
            |r| r.get(0),
        )
        .optional()?;
    if name.is_some() {
        c.execute(
            "UPDATE device_codes SET approved_by = ?2, approved_at = ?3 WHERE user_code = ?1",
            params![code, user_id, now],
        )?;
    }
    Ok(name)
}

/// The device's poll. An approved code yields a token once and is deleted.
pub fn poll(c: &mut Connection, device_code: &str, now: i64) -> Result<Poll> {
    type Row = (i64, Option<String>, Option<String>, Option<String>, Option<i64>);
    let tx = c.transaction()?;
    let hash = sha256_hex(device_code);
    let row: Option<Row> = tx
        .query_row(
            "SELECT expires_at, approved_by, device_id, device_name, last_poll FROM device_codes WHERE device_code_hash = ?1",
            [&hash],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let Some((expires, approved_by, device_id, device_name, last_poll)) = row else {
        return Ok(Poll::Expired);
    };
    if let Some(uid) = approved_by {
        let user = tx.query_row("SELECT id, name, is_admin FROM users WHERE id = ?1", [&uid], |r| {
            Ok(User {
                id: r.get(0)?,
                name: r.get(1)?,
                is_admin: r.get(2)?,
            })
        })?;
        let token = issue_token(&tx, &uid, device_id.as_deref(), device_name.as_deref())?;
        tx.execute("DELETE FROM device_codes WHERE device_code_hash = ?1", [&hash])?;
        tx.commit()?;
        return Ok(Poll::Approved { token, user });
    }
    if expires < now {
        tx.execute("DELETE FROM device_codes WHERE device_code_hash = ?1", [&hash])?;
        tx.commit()?;
        return Ok(Poll::Expired);
    }
    tx.execute(
        "UPDATE device_codes SET last_poll = ?2 WHERE device_code_hash = ?1",
        params![hash, now],
    )?;
    tx.commit()?;
    if last_poll.is_some_and(|p| now - p < INTERVAL_SECS - 1) {
        return Ok(Poll::SlowDown);
    }
    Ok(Poll::Pending)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    #[test]
    fn user_codes() {
        let c = new_user_code();
        assert_eq!(normalize_user_code(&c).as_deref(), Some(c.as_str()));
        assert_eq!(normalize_user_code("bcdf 1234").as_deref(), Some("BCDF-1234"));
        assert_eq!(normalize_user_code("BCDF1234").as_deref(), Some("BCDF-1234"));
        assert_eq!(normalize_user_code("BCD-1234"), None);
        assert_eq!(normalize_user_code("1234-BCDF"), None);
    }

    #[tokio::test]
    async fn flow_and_expiry() {
        let db = Db::memory().unwrap();
        let u = db.call(|c| super::super::add_user(c, "sam", "password123", false)).await.unwrap();
        let t0 = 1_000_000;
        let uid = u.id.clone();
        db.call(move |c| {
            let s = start(c, Some("tv1"), Some("Apple TV"), t0)?;
            assert!(s.device_code.starts_with("dc_"));
            // The device code itself is not stored.
            let n: i64 = c.query_row(
                "SELECT COUNT(*) FROM device_codes WHERE device_code_hash = ?1",
                [sha256_hex(&s.device_code)],
                |r| r.get(0),
            )?;
            assert_eq!(n, 1);
            assert_eq!(poll(c, &s.device_code, t0 + 1)?, Poll::Pending);
            assert_eq!(poll(c, &s.device_code, t0 + 2)?, Poll::SlowDown);
            assert_eq!(poll(c, "dc_wrong", t0 + 10)?, Poll::Expired);
            // A wrong code approves nothing; the right one (any spelling) does, once.
            assert_eq!(approve(c, "ZZZZ-0000", &uid, t0 + 20)?, None);
            assert_eq!(
                approve(c, &s.user_code.to_lowercase().replace('-', " "), &uid, t0 + 20)?,
                Some(Some("Apple TV".into()))
            );
            assert_eq!(approve(c, &s.user_code, &uid, t0 + 21)?, None);
            match poll(c, &s.device_code, t0 + 30)? {
                Poll::Approved { token, user } => {
                    assert!(token.starts_with("moonrise_"));
                    assert_eq!(user.id, uid);
                    let dev: String = c.query_row("SELECT device_name FROM tokens WHERE hash = ?1", [sha256_hex(&token)], |r| r.get(0))?;
                    assert_eq!(dev, "Apple TV");
                }
                other => panic!("{other:?}"),
            }
            // Used once.
            assert_eq!(poll(c, &s.device_code, t0 + 40)?, Poll::Expired);

            // Expiry: not approvable, and the poll says expired.
            let s = start(c, None, None, t0)?;
            assert_eq!(approve(c, &s.user_code, &uid, t0 + EXPIRES_SECS + 1)?, None);
            assert_eq!(poll(c, &s.device_code, t0 + EXPIRES_SECS + 1)?, Poll::Expired);
            assert_eq!(poll(c, &s.device_code, t0 + EXPIRES_SECS + 10)?, Poll::Expired);
            Ok(())
        })
        .await
        .unwrap();
    }
}
