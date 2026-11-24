//! Database access. Every query is checked at run time so builds need no database.

use sqlx::{PgConnection, PgPool};
use time::{Duration, OffsetDateTime};

use crate::names;
use crate::tokens;

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

pub const DEVICE_TOKEN_PREFIX: &str = "mgd_";
const SESSION_PREFIX: &str = "mgs_";
/// `accounts.kind` and `accounts.provider` of service accounts.
pub const SERVICE_KIND: &str = "service";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Account {
    pub id: i32,
    pub provider: String,
    pub subject: String,
    pub email: Option<String>,
    pub player_name: Option<String>,
    pub created_at: OffsetDateTime,
    pub banned_at: Option<OffsetDateTime>,
    pub ban_reason: Option<String>,
    /// `person` (signs in with Google) or `service` (see `create_service_account`).
    pub kind: String,
}

impl Account {
    pub fn is_banned(&self) -> bool {
        self.banned_at.is_some()
    }

    pub fn is_service(&self) -> bool {
        self.kind == SERVICE_KIND
    }
}

/// Account columns in `Account` order, prefixed and suffixed by literal SQL.
macro_rules! account_sql {
    ($head:literal, $tail:literal) => {
        concat!(
            $head,
            " id, provider, subject, email, player_name, created_at, banned_at, ban_reason, kind ",
            $tail
        )
    };
}

pub async fn upsert_identity(
    pool: &PgPool,
    provider: &str,
    subject: &str,
    email: Option<&str>,
) -> sqlx::Result<Account> {
    sqlx::query_as(account_sql!(
        "INSERT INTO accounts (provider, subject, email) VALUES ($1, $2, $3)
         ON CONFLICT (provider, subject) DO UPDATE SET email = EXCLUDED.email RETURNING",
        ""
    ))
    .bind(provider)
    .bind(subject)
    .bind(email)
    .fetch_one(pool)
    .await
}

pub async fn account(pool: &PgPool, id: i32) -> sqlx::Result<Option<Account>> {
    sqlx::query_as(account_sql!("SELECT", "FROM accounts WHERE id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn account_by_name(pool: &PgPool, name: &str) -> sqlx::Result<Option<Account>> {
    sqlx::query_as(account_sql!(
        "SELECT",
        "FROM accounts WHERE lower(player_name) = $1"
    ))
    .bind(names::name_key(name))
    .fetch_optional(pool)
    .await
}

/// `(name key, account id)` for each account whose player name has one of these keys
/// (lowercase names).
pub async fn account_ids_by_name_keys(
    pool: &PgPool,
    keys: &[String],
) -> sqlx::Result<Vec<(String, i32)>> {
    sqlx::query_as("SELECT lower(player_name), id FROM accounts WHERE lower(player_name) = ANY($1)")
        .bind(keys)
        .fetch_all(pool)
        .await
}

pub async fn accounts_by_email(pool: &PgPool, email: &str) -> sqlx::Result<Vec<Account>> {
    sqlx::query_as(account_sql!(
        "SELECT",
        "FROM accounts WHERE lower(email) = lower($1) ORDER BY id"
    ))
    .bind(email)
    .fetch_all(pool)
    .await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimOutcome {
    Claimed,
    Invalid,
    Taken,
    Reserved,
    AlreadyNamed,
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(d) if d.code().as_deref() == Some("23505"))
}

/// Serializes every change to who holds or reserves a name, until the transaction ends.
/// Without it, a claim could read "not reserved" while a deletion is about to reserve the
/// name, then win the unique index once the deleted row is gone.
pub async fn lock_name(conn: &mut PgConnection, name: &str) -> sqlx::Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('player-name:' || $1))")
        .bind(names::name_key(name))
        .execute(conn)
        .await?;
    Ok(())
}

/// Gives an unnamed account a free, unreserved name.
pub async fn claim_name(pool: &PgPool, account_id: i32, name: &str) -> sqlx::Result<ClaimOutcome> {
    if !names::valid_player_name(name) {
        return Ok(ClaimOutcome::Invalid);
    }
    let mut tx = pool.begin().await?;
    lock_name(&mut tx, name).await?;
    let reserved: Option<(String,)> =
        sqlx::query_as("SELECT name_key FROM reserved_names WHERE name_key = $1")
            .bind(names::name_key(name))
            .fetch_optional(&mut *tx)
            .await?;
    if reserved.is_some() {
        return Ok(ClaimOutcome::Reserved);
    }
    let result =
        sqlx::query("UPDATE accounts SET player_name = $2 WHERE id = $1 AND player_name IS NULL")
            .bind(account_id)
            .bind(name)
            .execute(&mut *tx)
            .await;
    let outcome = match result {
        Ok(r) if r.rows_affected() == 1 => ClaimOutcome::Claimed,
        Ok(_) => ClaimOutcome::AlreadyNamed,
        Err(e) if is_unique_violation(&e) => return Ok(ClaimOutcome::Taken),
        Err(e) => return Err(e),
    };
    tx.commit().await?;
    Ok(outcome)
}

/// Deletes the account, its sessions and devices, and keeps its name reserved so nobody
/// else inherits the characters stored under it.
pub async fn delete_account(pool: &PgPool, account_id: i32) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT player_name FROM accounts WHERE id = $1 FOR UPDATE")
            .bind(account_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((name,)) = row else {
        return Ok(false);
    };
    if let Some(name) = name {
        lock_name(&mut tx, &name).await?;
        sqlx::query(
            "INSERT INTO reserved_names (name_key, name, reason) VALUES ($1, $2, $3)
             ON CONFLICT (name_key) DO NOTHING",
        )
        .bind(names::name_key(&name))
        .bind(&name)
        .bind(format!("deleted account {account_id}"))
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("DELETE FROM accounts WHERE id = $1")
        .bind(account_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(true)
}

// ---- Sign-in flows and sessions ----

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LoginFlow {
    pub state: String,
    pub nonce: String,
    pub pkce_verifier: String,
    pub return_to: String,
}

pub async fn create_login_flow(pool: &PgPool, flow: &LoginFlow, ttl: Duration) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO login_flows (state, nonce, pkce_verifier, return_to, expires_at)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&flow.state)
    .bind(&flow.nonce)
    .bind(&flow.pkce_verifier)
    .bind(&flow.return_to)
    .bind(OffsetDateTime::now_utc() + ttl)
    .execute(pool)
    .await?;
    Ok(())
}

/// Removes and returns an unexpired flow: each one completes at most once.
pub async fn take_login_flow(pool: &PgPool, state: &str) -> sqlx::Result<Option<LoginFlow>> {
    sqlx::query_as(
        "DELETE FROM login_flows WHERE state = $1 AND expires_at > now()
         RETURNING state, nonce, pkce_verifier, return_to",
    )
    .bind(state)
    .fetch_optional(pool)
    .await
}

pub struct NewSession {
    pub cookie: String,
}

pub async fn create_session(
    pool: &PgPool,
    account_id: i32,
    ttl: Duration,
) -> sqlx::Result<NewSession> {
    let (cookie, hash) = tokens::new_secret(SESSION_PREFIX);
    let csrf = tokens::b64url(&tokens::random_bytes::<32>());
    sqlx::query(
        "INSERT INTO web_sessions (id_hash, account_id, csrf_token, expires_at)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(&hash[..])
    .bind(account_id)
    .bind(csrf)
    .bind(OffsetDateTime::now_utc() + ttl)
    .execute(pool)
    .await?;
    Ok(NewSession { cookie })
}

pub struct Session {
    pub account: Account,
    pub csrf_token: String,
    pub id_hash: [u8; 32],
}

pub async fn session(pool: &PgPool, cookie: &str) -> sqlx::Result<Option<Session>> {
    if !cookie.starts_with(SESSION_PREFIX) {
        return Ok(None);
    }
    let hash = tokens::sha256(cookie.as_bytes());
    let row: Option<(i32, String)> = sqlx::query_as(
        "SELECT account_id, csrf_token FROM web_sessions WHERE id_hash = $1 AND expires_at > now()",
    )
    .bind(&hash[..])
    .fetch_optional(pool)
    .await?;
    let Some((account_id, csrf_token)) = row else {
        return Ok(None);
    };
    // Service accounts never sign in to the site; nothing creates such a session, and none
    // would be honoured.
    Ok(account(pool, account_id)
        .await?
        .filter(|a| !a.is_service())
        .map(|account| Session {
            account,
            csrf_token,
            id_hash: hash,
        }))
}

pub async fn delete_session(pool: &PgPool, id_hash: &[u8; 32]) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM web_sessions WHERE id_hash = $1")
        .bind(&id_hash[..])
        .execute(pool)
        .await?;
    Ok(())
}

// ---- Launcher linking ----

pub struct NewDeviceCode {
    pub device_code: String,
    pub user_code: String,
}

/// Starts a link. Returns `None` when too many links are already waiting.
pub async fn create_device_code(
    pool: &PgPool,
    label: &str,
    ttl: Duration,
    max_pending: i64,
) -> sqlx::Result<Option<NewDeviceCode>> {
    let (pending,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM device_codes WHERE status = 'pending' AND expires_at > now()",
    )
    .fetch_one(pool)
    .await?;
    if pending >= max_pending {
        return Ok(None);
    }
    for _ in 0..8 {
        let (device_code, hash) = tokens::new_secret("mgl_");
        let user_code = tokens::new_user_code();
        let inserted = sqlx::query(
            "INSERT INTO device_codes (device_code_hash, user_code, client_label, expires_at)
             VALUES ($1, $2, $3, $4) ON CONFLICT (user_code) DO NOTHING",
        )
        .bind(&hash[..])
        .bind(&user_code)
        .bind(label)
        .bind(OffsetDateTime::now_utc() + ttl)
        .execute(pool)
        .await?;
        if inserted.rows_affected() == 1 {
            return Ok(Some(NewDeviceCode {
                device_code,
                user_code,
            }));
        }
        // An old code still holds this user code; clear expired ones and retry.
        sqlx::query("DELETE FROM device_codes WHERE expires_at <= now()")
            .execute(pool)
            .await?;
    }
    Ok(None)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PendingLink {
    pub user_code: String,
    pub client_label: String,
    pub created_at: OffsetDateTime,
}

pub async fn pending_link(pool: &PgPool, user_code: &str) -> sqlx::Result<Option<PendingLink>> {
    sqlx::query_as(
        "SELECT user_code, client_label, created_at FROM device_codes
         WHERE user_code = $1 AND status = 'pending' AND expires_at > now()",
    )
    .bind(user_code)
    .fetch_optional(pool)
    .await
}

/// Approves or denies a waiting link for this account. False if it is no longer waiting.
pub async fn decide_link(
    pool: &PgPool,
    user_code: &str,
    account_id: i32,
    approve: bool,
) -> sqlx::Result<bool> {
    let status = if approve { "approved" } else { "denied" };
    let r = sqlx::query(
        "UPDATE device_codes SET status = $3, account_id = $2
         WHERE user_code = $1 AND status = 'pending' AND expires_at > now()",
    )
    .bind(user_code)
    .bind(account_id)
    .bind(status)
    .execute(pool)
    .await?;
    Ok(r.rows_affected() == 1)
}

#[derive(Debug, PartialEq, Eq)]
pub enum PollOutcome {
    Pending,
    SlowDown,
    Denied,
    Expired,
    Invalid,
    Linked {
        device_token: String,
        player_name: Option<String>,
    },
}

/// status, account, launcher label, expiry, last poll.
type LinkRow = (
    String,
    Option<i32>,
    String,
    OffsetDateTime,
    Option<OffsetDateTime>,
);

pub async fn poll_device_code(
    pool: &PgPool,
    device_code: &str,
    min_interval: Duration,
) -> sqlx::Result<PollOutcome> {
    let hash = tokens::sha256(device_code.as_bytes());
    let mut tx = pool.begin().await?;
    let row: Option<LinkRow> = sqlx::query_as(
        "SELECT status, account_id, client_label, expires_at, last_poll_at FROM device_codes
             WHERE device_code_hash = $1 FOR UPDATE",
    )
    .bind(&hash[..])
    .fetch_optional(&mut *tx)
    .await?;
    let Some((status, account_id, label, expires_at, last_poll)) = row else {
        return Ok(PollOutcome::Invalid);
    };
    let now = OffsetDateTime::now_utc();
    if expires_at <= now {
        return Ok(PollOutcome::Expired);
    }
    sqlx::query("UPDATE device_codes SET last_poll_at = $2 WHERE device_code_hash = $1")
        .bind(&hash[..])
        .bind(now)
        .execute(&mut *tx)
        .await?;
    // Allow a second of network jitter before telling a launcher to slow down.
    let too_soon = last_poll.is_some_and(|t| now - t < min_interval - Duration::seconds(1));
    let outcome = match (status.as_str(), account_id) {
        (_, _) if too_soon && status == "pending" => PollOutcome::SlowDown,
        ("pending", _) => PollOutcome::Pending,
        ("denied", _) => {
            sqlx::query("DELETE FROM device_codes WHERE device_code_hash = $1")
                .bind(&hash[..])
                .execute(&mut *tx)
                .await?;
            PollOutcome::Denied
        }
        ("approved", Some(account_id)) => {
            let (device_token, token_hash) = tokens::new_secret(DEVICE_TOKEN_PREFIX);
            sqlx::query("INSERT INTO devices (account_id, token_hash, label) VALUES ($1, $2, $3)")
                .bind(account_id)
                .bind(&token_hash[..])
                .bind(&label)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE device_codes SET status = 'consumed' WHERE device_code_hash = $1")
                .bind(&hash[..])
                .execute(&mut *tx)
                .await?;
            let (player_name,): (Option<String>,) =
                sqlx::query_as("SELECT player_name FROM accounts WHERE id = $1")
                    .bind(account_id)
                    .fetch_one(&mut *tx)
                    .await?;
            PollOutcome::Linked {
                device_token,
                player_name,
            }
        }
        _ => PollOutcome::Invalid,
    };
    tx.commit().await?;
    Ok(outcome)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Device {
    pub id: i32,
    pub label: String,
    pub created_at: OffsetDateTime,
    pub last_used_at: Option<OffsetDateTime>,
}

pub async fn devices(pool: &PgPool, account_id: i32) -> sqlx::Result<Vec<Device>> {
    sqlx::query_as(
        "SELECT id, label, created_at, last_used_at FROM devices WHERE account_id = $1 ORDER BY id",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
}

pub async fn revoke_device(pool: &PgPool, account_id: i32, device_id: i32) -> sqlx::Result<bool> {
    let r = sqlx::query("DELETE FROM devices WHERE id = $1 AND account_id = $2")
        .bind(device_id)
        .bind(account_id)
        .execute(pool)
        .await?;
    Ok(r.rows_affected() == 1)
}

/// The account behind a device token, recording the use.
///
/// Recording the use is bookkeeping and only best effort: a database that answers reads but not
/// writes (a primary failing over, a busy one) still lets launchers get tickets.
pub async fn device_account(pool: &PgPool, device_token: &str) -> sqlx::Result<Option<Account>> {
    if !device_token.starts_with(DEVICE_TOKEN_PREFIX) {
        return Ok(None);
    }
    let hash = tokens::sha256(device_token.as_bytes());
    let row: Option<(i32, i32)> =
        sqlx::query_as("SELECT id, account_id FROM devices WHERE token_hash = $1")
            .bind(&hash[..])
            .fetch_optional(pool)
            .await?;
    let Some((device, account_id)) = row else {
        return Ok(None);
    };
    let found = account(pool, account_id).await?;
    if let Err(e) = sqlx::query("UPDATE devices SET last_used_at = now() WHERE id = $1")
        .bind(device)
        .execute(pool)
        .await
    {
        tracing::warn!(error = %e, device, "device use not recorded");
    }
    Ok(found)
}

// ---- Operator actions ----

pub async fn set_ban(pool: &PgPool, name: &str, reason: Option<&str>) -> sqlx::Result<bool> {
    let r = match reason {
        Some(reason) => sqlx::query(
            "UPDATE accounts SET banned_at = now(), ban_reason = $2 WHERE lower(player_name) = $1",
        )
        .bind(names::name_key(name))
        .bind(reason)
        .execute(pool)
        .await?,
        None => sqlx::query(
            "UPDATE accounts SET banned_at = NULL, ban_reason = NULL WHERE lower(player_name) = $1",
        )
        .bind(names::name_key(name))
        .execute(pool)
        .await?,
    };
    Ok(r.rows_affected() == 1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReserveOutcome {
    Reserved,
    AlreadyReserved,
    InUse,
    Invalid,
}

pub async fn reserve_name(pool: &PgPool, name: &str, reason: &str) -> sqlx::Result<ReserveOutcome> {
    if !names::valid_player_name(name) {
        return Ok(ReserveOutcome::Invalid);
    }
    let mut tx = pool.begin().await?;
    lock_name(&mut tx, name).await?;
    let held: Option<(i32,)> =
        sqlx::query_as("SELECT id FROM accounts WHERE lower(player_name) = $1")
            .bind(names::name_key(name))
            .fetch_optional(&mut *tx)
            .await?;
    if held.is_some() {
        return Ok(ReserveOutcome::InUse);
    }
    let r = sqlx::query(
        "INSERT INTO reserved_names (name_key, name, reason) VALUES ($1, $2, $3)
         ON CONFLICT (name_key) DO NOTHING",
    )
    .bind(names::name_key(name))
    .bind(name)
    .bind(reason)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(if r.rows_affected() == 1 {
        ReserveOutcome::Reserved
    } else {
        ReserveOutcome::AlreadyReserved
    })
}

pub async fn release_name(pool: &PgPool, name: &str) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    lock_name(&mut tx, name).await?;
    let r = sqlx::query("DELETE FROM reserved_names WHERE name_key = $1")
        .bind(names::name_key(name))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(r.rows_affected() == 1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOutcome {
    Assigned,
    NotReserved,
    NoSuchAccount,
    AccountNamed,
}

/// Hands a reserved name to an account that has none yet.
pub async fn assign_reserved_name(
    pool: &PgPool,
    name: &str,
    account_id: i32,
) -> sqlx::Result<AssignOutcome> {
    let mut tx = pool.begin().await?;
    lock_name(&mut tx, name).await?;
    let reserved: Option<(String,)> =
        sqlx::query_as("DELETE FROM reserved_names WHERE name_key = $1 RETURNING name")
            .bind(names::name_key(name))
            .fetch_optional(&mut *tx)
            .await?;
    let Some((stored,)) = reserved else {
        return Ok(AssignOutcome::NotReserved);
    };
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT player_name FROM accounts WHERE id = $1 FOR UPDATE")
            .bind(account_id)
            .fetch_optional(&mut *tx)
            .await?;
    match row {
        None => return Ok(AssignOutcome::NoSuchAccount),
        Some((Some(_),)) => return Ok(AssignOutcome::AccountNamed),
        Some((None,)) => {}
    }
    sqlx::query("UPDATE accounts SET player_name = $2 WHERE id = $1")
        .bind(account_id)
        .bind(&stored)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(AssignOutcome::Assigned)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ReservedName {
    pub name: String,
    pub reason: String,
    pub created_at: OffsetDateTime,
}

pub async fn reserved_names(pool: &PgPool) -> sqlx::Result<Vec<ReservedName>> {
    sqlx::query_as("SELECT name, reason, created_at FROM reserved_names ORDER BY name_key")
        .fetch_all(pool)
        .await
}

// ---- Service accounts ----

/// The longest label a minted launcher token may carry, as for linked launchers.
pub const MAX_SERVICE_LABEL_LEN: usize = 64;

/// A label for a minted token: printable ASCII (spaces allowed), 1 to 64 characters once
/// trimmed. `None` when it is empty, too long or holds anything else.
pub fn service_token_label(raw: &str) -> Option<String> {
    let label = raw.trim();
    (!label.is_empty()
        && label.len() <= MAX_SERVICE_LABEL_LEN
        && label.bytes().all(|b| b == b' ' || b.is_ascii_graphic()))
    .then(|| label.to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceCreateOutcome {
    /// Created; `released` says whether a reservation of the name was given up for it.
    Created {
        account_id: i32,
        name: String,
        released: bool,
    },
    Invalid,
    /// An account (person or service) already holds the name.
    Taken,
}

/// Creates a service account holding `name`. The name must be valid and not held by any
/// account; a reserved name is taken over, releasing its reservation in the same transaction
/// under the name lock (the stored spelling of a reserved name is kept).
pub async fn create_service_account(
    pool: &PgPool,
    name: &str,
) -> sqlx::Result<ServiceCreateOutcome> {
    if !names::valid_player_name(name) {
        return Ok(ServiceCreateOutcome::Invalid);
    }
    let mut tx = pool.begin().await?;
    lock_name(&mut tx, name).await?;
    let held: Option<(i32,)> =
        sqlx::query_as("SELECT id FROM accounts WHERE lower(player_name) = $1")
            .bind(names::name_key(name))
            .fetch_optional(&mut *tx)
            .await?;
    if held.is_some() {
        return Ok(ServiceCreateOutcome::Taken);
    }
    let reserved: Option<(String,)> =
        sqlx::query_as("DELETE FROM reserved_names WHERE name_key = $1 RETURNING name")
            .bind(names::name_key(name))
            .fetch_optional(&mut *tx)
            .await?;
    let released = reserved.is_some();
    let stored = reserved.map_or_else(|| name.to_owned(), |(n,)| n);
    // A random subject under its own provider: no Google identity can ever upsert into it.
    let subject = format!("svc_{}", tokens::b64url(&tokens::random_bytes::<24>()));
    let result = sqlx::query_as(
        "INSERT INTO accounts (provider, subject, player_name, kind) VALUES ($1, $2, $3, $1)
         RETURNING id",
    )
    .bind(SERVICE_KIND)
    .bind(&subject)
    .bind(&stored)
    .fetch_one(&mut *tx)
    .await;
    let (account_id,): (i32,) = match result {
        Ok(row) => row,
        Err(e) if is_unique_violation(&e) => return Ok(ServiceCreateOutcome::Taken),
        Err(e) => return Err(e),
    };
    tx.commit().await?;
    Ok(ServiceCreateOutcome::Created {
        account_id,
        name: stored,
        released,
    })
}

pub async fn service_accounts(pool: &PgPool) -> sqlx::Result<Vec<Account>> {
    sqlx::query_as(account_sql!(
        "SELECT",
        "FROM accounts WHERE kind = 'service' ORDER BY lower(player_name)"
    ))
    .fetch_all(pool)
    .await
}

/// The service account holding `name`; `None` when nobody or a person holds it.
pub async fn service_account_by_name(pool: &PgPool, name: &str) -> sqlx::Result<Option<Account>> {
    Ok(account_by_name(pool, name)
        .await?
        .filter(Account::is_service))
}

pub struct MintedToken {
    pub device_id: i32,
    /// The `mgd_` token. Only its hash is stored; show it once.
    pub token: String,
}

/// Mints a launcher token for a service account, stored hashed like every linked launcher.
/// `None` when the account is not a service account.
pub async fn mint_service_token(
    pool: &PgPool,
    account_id: i32,
    label: &str,
) -> sqlx::Result<Option<MintedToken>> {
    let (token, hash) = tokens::new_secret(DEVICE_TOKEN_PREFIX);
    let row: Option<(i32,)> = sqlx::query_as(
        "INSERT INTO devices (account_id, token_hash, label)
         SELECT id, $2, $3 FROM accounts WHERE id = $1 AND kind = 'service' RETURNING id",
    )
    .bind(account_id)
    .bind(&hash[..])
    .bind(label)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(device_id,)| MintedToken { device_id, token }))
}

/// Drops expired sign-in flows, sessions, link codes and presence.
pub async fn cleanup(pool: &PgPool) -> sqlx::Result<u64> {
    let mut removed = 0;
    for sql in [
        "DELETE FROM login_flows WHERE expires_at <= now()",
        "DELETE FROM web_sessions WHERE expires_at <= now()",
        "DELETE FROM device_codes WHERE expires_at <= now()",
    ] {
        removed += sqlx::query(sql).execute(pool).await?.rows_affected();
    }
    removed += crate::social::purge_presence(pool).await?;
    Ok(removed)
}
