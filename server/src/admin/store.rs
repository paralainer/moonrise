//! The queries of Manage Server's accounts API (`manage.rs`): accounts, their devices (tokens),
//! invitations, and the names the activity page shows. The handlers check the request, call one
//! of these inside `Db::call`, and shape the answer.

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use crate::auth;

/// A token's public name: the start of its hash.
pub(crate) fn device_id(hash: &str) -> String {
    hash.chars().take(16).collect()
}

// ---------------------------------------------------------------- accounts

/// The accounts, admins first, with their devices and when one was last used (`me`: the
/// account asking).
pub(crate) fn list_users(c: &Connection, me: Option<&str>) -> Result<Vec<Value>> {
    let mut s = c.prepare(
        "SELECT u.id, u.name, u.is_admin, u.created_at,
                        (SELECT COUNT(*) FROM tokens t WHERE t.user_id = u.id AND t.revoked_at IS NULL),
                        (SELECT MAX(t.last_used) FROM tokens t WHERE t.user_id = u.id)
                 FROM users u ORDER BY u.is_admin DESC, u.name COLLATE NOCASE",
    )?;
    let rows = s
        .query_map([], |r| {
            let id: String = r.get(0)?;
            Ok(json!({"id": id, "name": r.get::<_, String>(1)?, "is_admin": r.get::<_, bool>(2)?, "created_at": r.get::<_, Option<i64>>(3)?.map(crate::library::rfc3339),
                      "devices": r.get::<_, i64>(4)?, "last_used": r.get::<_, Option<i64>>(5)?.map(crate::library::rfc3339), "me": me == Some(id.as_str())}))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The accounts: (id, name, is_admin).
pub(crate) fn users(c: &Connection) -> Result<Vec<(String, String, bool)>> {
    let mut s = c.prepare("SELECT id, name, is_admin FROM users")?;
    let r = s
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(r)
}

/// Gives or takes away an account's admin rights.
pub(crate) fn set_admin(c: &Connection, id: &str, admin: bool) -> Result<usize> {
    Ok(c.execute("UPDATE users SET is_admin = ?2 WHERE id = ?1", params![id, admin])?)
}

/// One account.
pub(crate) fn user(c: &Connection, id: &str) -> Result<auth::User> {
    Ok(c.query_row("SELECT id, name, is_admin FROM users WHERE id = ?1", [id], |r| {
        Ok(auth::User {
            id: r.get(0)?,
            name: r.get(1)?,
            is_admin: r.get(2)?,
        })
    })?)
}

/// Deletes an account and what belongs to it, in one transaction.
pub(crate) fn delete_user(c: &mut Connection, id: &str) -> Result<()> {
    let tx = c.transaction()?;
    // Foreign keys may be off: what belongs to the account goes explicitly.
    for t in ["user_data", "play_sessions", "tokens"] {
        tx.execute(&format!("DELETE FROM {t} WHERE user_id = ?1"), [id])?;
    }
    tx.execute("DELETE FROM device_codes WHERE approved_by = ?1", [id])?;
    tx.execute("DELETE FROM users WHERE id = ?1", [id])?;
    tx.commit()?;
    Ok(())
}

// ---------------------------------------------------------------- devices

/// An account's signed-in devices, the most recently used first (`current`: the token with the
/// hash `mine`).
pub(crate) fn devices(c: &Connection, user: &str, mine: Option<&str>) -> Result<Vec<Value>> {
    let mut s = c.prepare(
        "SELECT hash, device_name, device_id, created_at, last_used FROM tokens WHERE user_id = ?1 AND revoked_at IS NULL
                 ORDER BY COALESCE(last_used, created_at) DESC",
    )?;
    let rows = s
        .query_map([user], |r| {
            let hash: String = r.get(0)?;
            Ok(json!({"id": device_id(&hash), "device_name": r.get::<_, Option<String>>(1)?, "device_id": r.get::<_, Option<String>>(2)?,
                      "created_at": r.get::<_, Option<i64>>(3)?.map(crate::library::rfc3339), "last_used": r.get::<_, Option<i64>>(4)?.map(crate::library::rfc3339),
                      "current": mine == Some(hash.as_str())}))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Revokes a user's tokens whose hash starts with `prefix` (all: None), never `keep`. Answers
/// the hashes revoked.
pub(crate) fn revoke(c: &Connection, user: &str, prefix: Option<&str>, keep: Option<&str>, now: i64) -> Result<Vec<String>> {
    let mut s = c.prepare("SELECT hash FROM tokens WHERE user_id = ?1 AND revoked_at IS NULL")?;
    let all = s.query_map([user], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    let chosen: Vec<String> = all
        .into_iter()
        .filter(|h| prefix.is_none_or(|p| h.starts_with(p)) && keep != Some(h.as_str()))
        .collect();
    for h in &chosen {
        c.execute("UPDATE tokens SET revoked_at = ?2 WHERE hash = ?1", params![h, now])?;
    }
    Ok(chosen)
}

// ---------------------------------------------------------------- invitations

/// A new invitation (its code's hash), after dropping the used and expired ones.
pub(crate) fn add_invite(c: &Connection, id: &str, code_hash: &str, by: Option<&str>, now: i64, expires: i64, admin: bool) -> Result<()> {
    c.execute("DELETE FROM invites WHERE expires_at < ?1 OR used_at IS NOT NULL", [now])?;
    c.execute(
        "INSERT INTO invites(id, code_hash, created_by, created_at, expires_at, admin) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, code_hash, by, now, expires, admin],
    )?;
    Ok(())
}

/// The invitations still open, newest first (never their codes).
pub(crate) fn invites(c: &Connection, now: i64) -> Result<Vec<Value>> {
    let mut s = c.prepare(
        "SELECT i.id, i.admin, i.created_at, i.expires_at, u.name FROM invites i LEFT JOIN users u ON u.id = i.created_by
                 WHERE i.used_at IS NULL AND i.expires_at > ?1 ORDER BY i.created_at DESC",
    )?;
    let rows = s
        .query_map([now], |r| {
            Ok(
                json!({"id": r.get::<_, String>(0)?, "admin": r.get::<_, bool>(1)?, "created_at": crate::library::rfc3339(r.get(2)?),
                      "expires_at": crate::library::rfc3339(r.get(3)?), "created_by": r.get::<_, Option<String>>(4)?}),
            )
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Deletes an invitation: how many there were (0 or 1).
pub(crate) fn delete_invite(c: &Connection, id: &str) -> Result<usize> {
    Ok(c.execute("DELETE FROM invites WHERE id = ?1", [id])?)
}

/// Why an invitation wasn't redeemed.
#[derive(Debug, PartialEq)]
pub(crate) enum InviteRefused {
    /// No open invitation has that code.
    Invalid,
    /// The user name is taken.
    NameTaken,
}

/// An account made with an invitation (the code's hash) and signed in, in one transaction: the
/// account and its token. An error: the account couldn't be made (the name or password).
pub(crate) fn redeem_invite(
    c: &mut Connection,
    code_hash: &str,
    now: i64,
    name: &str,
    password: &str,
    device: (Option<&str>, Option<&str>),
) -> Result<Result<(auth::User, String), InviteRefused>> {
    let (device_id, device_name) = device;
    let tx = c.transaction()?;
    let inv: Option<(String, bool)> = tx
        .query_row(
            "SELECT id, admin FROM invites WHERE code_hash = ?1 AND used_at IS NULL AND expires_at > ?2",
            params![code_hash, now],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((inv_id, is_admin)) = inv else {
        return Ok(Err(InviteRefused::Invalid));
    };
    if auth::user_by_name(&tx, name)?.is_some() {
        return Ok(Err(InviteRefused::NameTaken));
    }
    let u = auth::add_user(&tx, name, password, is_admin)?;
    tx.execute(
        "UPDATE invites SET used_at = ?2, used_by = ?3 WHERE id = ?1",
        params![inv_id, now, u.id],
    )?;
    let token = auth::issue_token(&tx, &u.id, device_id, device_name)?;
    tx.commit()?;
    Ok(Ok((u, token)))
}

// ---------------------------------------------------------------- activity

/// A session's title: (title, series title, season, episode, kind).
pub(crate) type ItemNames = (String, Option<String>, Option<i64>, Option<i64>, String);

/// What a session last reported: (position, when).
pub(crate) type Reported = (Option<f64>, Option<i64>);

/// The names the activity page shows for sessions given as (user, item, session id): the user's
/// name, the title's, and where the session last reported.
pub(crate) fn session_names(
    c: &Connection,
    ids: Vec<(Option<String>, Option<String>, String)>,
) -> Result<Vec<(Option<String>, Option<ItemNames>, Option<Reported>)>> {
    let mut out = Vec::new();
    for (u, i, sid) in ids {
        let user: Option<String> = match &u {
            Some(u) => c.query_row("SELECT name FROM users WHERE id = ?1", [u], |r| r.get(0)).optional()?,
            None => None,
        };
        let item: Option<ItemNames> = match &i {
            Some(i) => c
                .query_row(
                    "SELECT i.title, s.title, i.season_no, i.episode_no, i.kind FROM items i LEFT JOIN items s ON s.id = i.series_id WHERE i.id = ?1",
                    [i],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
                .optional()?,
            None => None,
        };
        let pos: Option<Reported> = c
            .query_row("SELECT position, last_report_at FROM play_sessions WHERE id = ?1", [&sid], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        out.push((user, item, pos));
    }
    Ok(out)
}
