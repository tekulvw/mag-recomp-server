//! Site staff roles, the queries the admin panel needs, and its audit log.
//!
//! Roles rank owner > admin > moderator > everyone else. Staff may act on accounts and
//! grant roles strictly below their own, so nobody can promote themselves or touch a peer.

use sqlx::PgPool;
use time::OffsetDateTime;

use crate::db::Account;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Moderator,
    Admin,
    Owner,
}

impl Role {
    /// The stored name; owners are never stored.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Moderator => "moderator",
            Role::Admin => "admin",
            Role::Owner => "owner",
        }
    }

    /// A role that can be granted from the site.
    pub fn grantable(value: &str) -> Option<Role> {
        match value {
            "moderator" => Some(Role::Moderator),
            "admin" => Some(Role::Admin),
            _ => None,
        }
    }
}

/// True when `actor` may act on someone holding `target` (`None`: no role).
pub fn outranks(actor: Role, target: Option<Role>) -> bool {
    target.is_none_or(|t| actor > t)
}

pub fn email_key(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

/// A plausible email address: one `@`, something on both sides, no spaces, at most 254 bytes.
pub fn plausible_email(email: &str) -> bool {
    let e = email.trim();
    e.len() <= 254
        && !e.contains(char::is_whitespace)
        && e.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty() && domain.contains('.') && !domain.contains('@')
        })
}

pub fn parse_owner_emails(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(email_key)
        .filter(|e| !e.is_empty())
        .collect()
}

/// The role of whoever signed in with this (verified) email.
pub async fn role_for_email(
    pool: &PgPool,
    owners: &[String],
    email: Option<&str>,
) -> sqlx::Result<Option<Role>> {
    let Some(email) = email else {
        return Ok(None);
    };
    let key = email_key(email);
    if owners.contains(&key) {
        return Ok(Some(Role::Owner));
    }
    let row: Option<(String,)> = sqlx::query_as("SELECT role FROM staff WHERE email_key = $1")
        .bind(&key)
        .fetch_optional(pool)
        .await?;
    Ok(row.and_then(|(r,)| Role::grantable(&r)))
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct StaffRow {
    pub email_key: String,
    pub role: String,
    pub granted_by_name: Option<String>,
    pub created_at: OffsetDateTime,
}

pub async fn staff(pool: &PgPool) -> sqlx::Result<Vec<StaffRow>> {
    sqlx::query_as(
        "SELECT s.email_key, s.role, a.player_name AS granted_by_name, s.created_at
         FROM staff s LEFT JOIN accounts a ON a.id = s.granted_by
         ORDER BY s.role, s.email_key",
    )
    .fetch_all(pool)
    .await
}

pub async fn set_staff(
    pool: &PgPool,
    email: &str,
    role: Role,
    granted_by: i32,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO staff (email_key, role, granted_by) VALUES ($1, $2, $3)
         ON CONFLICT (email_key) DO UPDATE SET role = EXCLUDED.role, granted_by = EXCLUDED.granted_by",
    )
    .bind(email_key(email))
    .bind(role.as_str())
    .bind(granted_by)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn remove_staff(pool: &PgPool, email: &str) -> sqlx::Result<bool> {
    let r = sqlx::query("DELETE FROM staff WHERE email_key = $1")
        .bind(email_key(email))
        .execute(pool)
        .await?;
    Ok(r.rows_affected() == 1)
}

/// The stored (not owner) role for an email.
pub async fn stored_role(pool: &PgPool, email: &str) -> sqlx::Result<Option<Role>> {
    let row: Option<(String,)> = sqlx::query_as("SELECT role FROM staff WHERE email_key = $1")
        .bind(email_key(email))
        .fetch_optional(pool)
        .await?;
    Ok(row.and_then(|(r,)| Role::grantable(&r)))
}

// ---- Audit log ----

/// Writes one audit log entry, on the pool or inside a caller's transaction.
pub async fn log<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    actor: &Account,
    action: &str,
    target: &str,
    detail: &str,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO admin_log (actor_account, actor_email, action, target, detail)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(actor.id)
    .bind(actor.email.as_deref().unwrap_or(""))
    .bind(action)
    .bind(target)
    .bind(detail)
    .execute(executor)
    .await?;
    Ok(())
}

/// The actor recorded for `mag-portal admin` commands, which run without a site session.
pub const OPERATOR_ACTOR: &str = "operator (command line)";

/// An audit log entry for a change made with `mag-portal admin`.
pub async fn log_operator(
    pool: &PgPool,
    action: &str,
    target: &str,
    detail: &str,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO admin_log (actor_account, actor_email, action, target, detail)
         VALUES (NULL, $1, $2, $3, $4)",
    )
    .bind(OPERATOR_ACTOR)
    .bind(action)
    .bind(target)
    .bind(detail)
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LogRow {
    pub actor_email: String,
    pub action: String,
    pub target: String,
    pub detail: String,
    pub created_at: OffsetDateTime,
}

pub async fn recent_log(pool: &PgPool, limit: i64) -> sqlx::Result<Vec<LogRow>> {
    sqlx::query_as(
        "SELECT actor_email, action, target, detail, created_at FROM admin_log
         ORDER BY id DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}

// ---- Accounts for the users panel ----

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AccountSummary {
    pub id: i32,
    pub email: Option<String>,
    pub player_name: Option<String>,
    pub created_at: OffsetDateTime,
    pub banned_at: Option<OffsetDateTime>,
    pub devices: i64,
    pub kind: String,
}

/// `\`, `%` and `_` taken literally inside a LIKE pattern.
pub(crate) fn like_literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Accounts whose name or email contains `query` (any case), or whose ID equals it; newest first.
pub async fn search_accounts(
    pool: &PgPool,
    query: &str,
    limit: i64,
    offset: i64,
) -> sqlx::Result<Vec<AccountSummary>> {
    let q = query.trim();
    let pattern = format!("%{}%", like_literal(&q.to_ascii_lowercase()));
    let id: Option<i32> = q.parse().ok();
    sqlx::query_as(
        "SELECT a.id, a.email, a.player_name, a.created_at, a.banned_at,
                (SELECT count(*) FROM devices d WHERE d.account_id = a.id) AS devices, a.kind
         FROM accounts a
         WHERE lower(coalesce(a.player_name, '')) LIKE $1 OR lower(coalesce(a.email, '')) LIKE $1
               OR a.id = $2
         ORDER BY a.id DESC LIMIT $3 OFFSET $4",
    )
    .bind(pattern)
    .bind(id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
}

pub struct Stats {
    pub accounts: i64,
    pub named: i64,
    pub banned: i64,
    pub devices: i64,
    pub reserved: i64,
}

pub async fn stats(pool: &PgPool) -> sqlx::Result<Stats> {
    let (accounts, named, banned, devices, reserved): (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM accounts),
                (SELECT count(*) FROM accounts WHERE player_name IS NOT NULL),
                (SELECT count(*) FROM accounts WHERE banned_at IS NOT NULL),
                (SELECT count(*) FROM devices),
                (SELECT count(*) FROM reserved_names)",
    )
    .fetch_one(pool)
    .await?;
    Ok(Stats {
        accounts,
        named,
        banned,
        devices,
        reserved,
    })
}

pub async fn set_ban_by_id(
    pool: &PgPool,
    account_id: i32,
    reason: Option<&str>,
) -> sqlx::Result<bool> {
    let r = match reason {
        Some(reason) => {
            sqlx::query("UPDATE accounts SET banned_at = now(), ban_reason = $2 WHERE id = $1")
                .bind(account_id)
                .bind(reason)
                .execute(pool)
                .await?
        }
        None => {
            sqlx::query("UPDATE accounts SET banned_at = NULL, ban_reason = NULL WHERE id = $1")
                .bind(account_id)
                .execute(pool)
                .await?
        }
    };
    Ok(r.rows_affected() == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranking() {
        assert!(outranks(Role::Moderator, None));
        assert!(!outranks(Role::Moderator, Some(Role::Moderator)));
        assert!(outranks(Role::Admin, Some(Role::Moderator)));
        assert!(!outranks(Role::Admin, Some(Role::Admin)));
        assert!(outranks(Role::Owner, Some(Role::Admin)));
        assert!(!outranks(Role::Owner, Some(Role::Owner)));
        assert_eq!(Role::grantable("owner"), None);
    }

    #[test]
    fn emails() {
        assert_eq!(
            parse_owner_emails(" A@Example.test, ,b@x.y "),
            vec!["a@example.test", "b@x.y"]
        );
        assert!(plausible_email("someone@example.test"));
        for bad in [
            "",
            "no-at",
            "@example.test",
            "a@nodot",
            "a b@x.y",
            "a@b@c.d",
        ] {
            assert!(!plausible_email(bad), "{bad}");
        }
        assert_eq!(like_literal("a_b%c\\"), "a\\_b\\%c\\\\");
    }
}
