//! Friends, friend requests, blocks and the game presence relay (docs/portal-social.md).
//!
//! Friendships are mutual: one player asks, the other accepts (or asks back). Presence is the
//! latest blob a player's game published, kept for `PRESENCE_TTL_SECONDS` and shown only to
//! mutual friends. The portal never looks inside it.
//!
//! Every change that reads before it writes holds both accounts' social locks, taken in account
//! order, so racing requests, accepts and blocks between the same players (and the per-account
//! limits) stay consistent.

use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;

use crate::names;

/// Seconds a presence update counts as online.
pub const PRESENCE_TTL_SECONDS: i64 = 60;
/// The largest presence blob, in bytes.
pub const MAX_PRESENCE_BYTES: usize = 1024;
/// How often clients should poll `GET /api/v1/social`.
pub const POLL_SECONDS: u32 = 15;
pub const MAX_FRIENDS: i64 = 200;
/// Outgoing requests waiting for an answer.
pub const MAX_OUTGOING: i64 = 50;
pub const MAX_BLOCKS: i64 = 1000;

/// The advisory-lock key space of social changes: `pg_advisory_xact_lock(LOCK_SPACE, account)`.
/// Two-key advisory locks never collide with the one-key name locks (`db::lock_name`).
const LOCK_SPACE: i32 = 0x534f_4331;

/// Another player, as the social lists name them.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Peer {
    pub id: i32,
    /// The canonical spelling of their player name.
    pub name: String,
}

/// The named account, matched case-insensitively. `None` for names that are not valid player
/// names or that nobody holds.
pub async fn find(pool: &PgPool, name: &str) -> sqlx::Result<Option<Peer>> {
    if !names::valid_player_name(name) {
        return Ok(None);
    }
    sqlx::query_as("SELECT id, player_name AS name FROM accounts WHERE lower(player_name) = $1")
        .bind(names::name_key(name))
        .fetch_optional(pool)
        .await
}

async fn lock_pair(conn: &mut PgConnection, a: i32, b: i32) -> sqlx::Result<()> {
    for id in [a.min(b), a.max(b)] {
        sqlx::query("SELECT pg_advisory_xact_lock($1, $2)")
            .bind(LOCK_SPACE)
            .bind(id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

async fn exists(conn: &mut PgConnection, sql: &'static str, a: i32, b: i32) -> sqlx::Result<bool> {
    let (found,): (bool,) = sqlx::query_as(sql).bind(a).bind(b).fetch_one(conn).await?;
    Ok(found)
}

async fn has_blocked(conn: &mut PgConnection, blocker: i32, blocked: i32) -> sqlx::Result<bool> {
    exists(
        conn,
        "SELECT EXISTS (SELECT 1 FROM blocks WHERE blocker = $1 AND blocked = $2)",
        blocker,
        blocked,
    )
    .await
}

async fn has_requested(conn: &mut PgConnection, from: i32, to: i32) -> sqlx::Result<bool> {
    exists(
        conn,
        "SELECT EXISTS (SELECT 1 FROM friend_requests WHERE from_account = $1 AND to_account = $2)",
        from,
        to,
    )
    .await
}

async fn are_friends(conn: &mut PgConnection, a: i32, b: i32) -> sqlx::Result<bool> {
    exists(
        conn,
        "SELECT EXISTS (SELECT 1 FROM friendships
                        WHERE account_lo = least($1, $2) AND account_hi = greatest($1, $2))",
        a,
        b,
    )
    .await
}

async fn count(conn: &mut PgConnection, sql: &'static str, id: i32) -> sqlx::Result<i64> {
    let (n,): (i64,) = sqlx::query_as(sql).bind(id).fetch_one(conn).await?;
    Ok(n)
}

const COUNT_FRIENDS: &str =
    "SELECT count(*) FROM friendships WHERE account_lo = $1 OR account_hi = $1";
const COUNT_OUTGOING: &str = "SELECT count(*) FROM friend_requests WHERE from_account = $1";
const COUNT_BLOCKS: &str = "SELECT count(*) FROM blocks WHERE blocker = $1";

/// Makes `a` and `b` friends and drops the requests between them.
async fn befriend(conn: &mut PgConnection, a: i32, b: i32) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO friendships (account_lo, account_hi) VALUES (least($1, $2), greatest($1, $2))
         ON CONFLICT DO NOTHING",
    )
    .bind(a)
    .bind(b)
    .execute(&mut *conn)
    .await?;
    drop_requests(conn, a, b).await
}

async fn drop_requests(conn: &mut PgConnection, a: i32, b: i32) -> sqlx::Result<()> {
    sqlx::query(
        "DELETE FROM friend_requests
         WHERE (from_account = $1 AND to_account = $2) OR (from_account = $2 AND to_account = $1)",
    )
    .bind(a)
    .bind(b)
    .execute(conn)
    .await?;
    Ok(())
}

async fn friends_full(conn: &mut PgConnection, a: i32, b: i32) -> sqlx::Result<bool> {
    Ok(count(conn, COUNT_FRIENDS, a).await? >= MAX_FRIENDS
        || count(conn, COUNT_FRIENDS, b).await? >= MAX_FRIENDS)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestOutcome {
    /// Waiting for them (also when they block the sender: see `request`).
    Requested,
    /// They had already asked, so the two are now friends.
    Accepted,
    AlreadyFriends,
    /// The sender blocks them.
    Blocked,
    /// One side already has `MAX_FRIENDS` friends.
    FriendLimit,
    /// The sender already has `MAX_OUTGOING` requests waiting.
    RequestLimit,
}

/// `from` asks `to` to be friends. Asking again is harmless. A request to someone who blocks
/// the sender is stored and answered `Requested` like any other, but they never see it, so a
/// block cannot be detected (docs/portal-social.md).
pub async fn request(pool: &PgPool, from: i32, to: i32) -> sqlx::Result<RequestOutcome> {
    let mut tx = pool.begin().await?;
    lock_pair(&mut tx, from, to).await?;
    if has_blocked(&mut tx, from, to).await? {
        return Ok(RequestOutcome::Blocked);
    }
    if are_friends(&mut tx, from, to).await? {
        return Ok(RequestOutcome::AlreadyFriends);
    }
    if !has_blocked(&mut tx, to, from).await? && has_requested(&mut tx, to, from).await? {
        if friends_full(&mut tx, from, to).await? {
            return Ok(RequestOutcome::FriendLimit);
        }
        befriend(&mut tx, from, to).await?;
        tx.commit().await?;
        return Ok(RequestOutcome::Accepted);
    }
    if has_requested(&mut tx, from, to).await? {
        return Ok(RequestOutcome::Requested);
    }
    if count(&mut tx, COUNT_FRIENDS, from).await? >= MAX_FRIENDS {
        return Ok(RequestOutcome::FriendLimit);
    }
    if count(&mut tx, COUNT_OUTGOING, from).await? >= MAX_OUTGOING {
        return Ok(RequestOutcome::RequestLimit);
    }
    sqlx::query("INSERT INTO friend_requests (from_account, to_account) VALUES ($1, $2)")
        .bind(from)
        .bind(to)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(RequestOutcome::Requested)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptOutcome {
    Accepted,
    /// No request from them is waiting (or `me` blocks them, which hides it).
    NoRequest,
    FriendLimit,
}

/// `me` accepts the request `from` sent.
pub async fn accept(pool: &PgPool, me: i32, from: i32) -> sqlx::Result<AcceptOutcome> {
    let mut tx = pool.begin().await?;
    lock_pair(&mut tx, me, from).await?;
    if has_blocked(&mut tx, me, from).await? || !has_requested(&mut tx, from, me).await? {
        return Ok(AcceptOutcome::NoRequest);
    }
    if friends_full(&mut tx, me, from).await? {
        return Ok(AcceptOutcome::FriendLimit);
    }
    befriend(&mut tx, me, from).await?;
    tx.commit().await?;
    Ok(AcceptOutcome::Accepted)
}

/// Removes the request `from` sent to `to`; false when there was none. Declining (`to` acts) and
/// cancelling (`from` acts) are the same change.
pub async fn withdraw(pool: &PgPool, from: i32, to: i32) -> sqlx::Result<bool> {
    let r = sqlx::query("DELETE FROM friend_requests WHERE from_account = $1 AND to_account = $2")
        .bind(from)
        .bind(to)
        .execute(pool)
        .await?;
    Ok(r.rows_affected() == 1)
}

/// Ends a friendship for both players; false when they were not friends.
pub async fn unfriend(pool: &PgPool, a: i32, b: i32) -> sqlx::Result<bool> {
    let r = sqlx::query(
        "DELETE FROM friendships WHERE account_lo = least($1, $2) AND account_hi = greatest($1, $2)",
    )
    .bind(a)
    .bind(b)
    .execute(pool)
    .await?;
    Ok(r.rows_affected() == 1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockOutcome {
    Blocked,
    /// `me` already blocks `MAX_BLOCKS` players.
    Limit,
}

/// `me` blocks `other`, ending their friendship and every request between them.
pub async fn block(pool: &PgPool, me: i32, other: i32) -> sqlx::Result<BlockOutcome> {
    let mut tx = pool.begin().await?;
    lock_pair(&mut tx, me, other).await?;
    if !has_blocked(&mut tx, me, other).await? {
        if count(&mut tx, COUNT_BLOCKS, me).await? >= MAX_BLOCKS {
            return Ok(BlockOutcome::Limit);
        }
        sqlx::query("INSERT INTO blocks (blocker, blocked) VALUES ($1, $2)")
            .bind(me)
            .bind(other)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query(
        "DELETE FROM friendships WHERE account_lo = least($1, $2) AND account_hi = greatest($1, $2)",
    )
    .bind(me)
    .bind(other)
    .execute(&mut *tx)
    .await?;
    drop_requests(&mut tx, me, other).await?;
    tx.commit().await?;
    Ok(BlockOutcome::Blocked)
}

/// False when `me` did not block `other`.
pub async fn unblock(pool: &PgPool, me: i32, other: i32) -> sqlx::Result<bool> {
    let r = sqlx::query("DELETE FROM blocks WHERE blocker = $1 AND blocked = $2")
        .bind(me)
        .bind(other)
        .execute(pool)
        .await?;
    Ok(r.rows_affected() == 1)
}

/// Stores the player's latest presence (at most `MAX_PRESENCE_BYTES`), replacing the last one.
pub async fn set_presence(pool: &PgPool, account_id: i32, data: &[u8]) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO presence (account_id, data, updated_at) VALUES ($1, $2, now())
         ON CONFLICT (account_id) DO UPDATE SET data = EXCLUDED.data, updated_at = now()",
    )
    .bind(account_id)
    .bind(data)
    .execute(pool)
    .await?;
    Ok(())
}

/// Forgets the player's presence: they are offline.
pub async fn clear_presence(pool: &PgPool, account_id: i32) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM presence WHERE account_id = $1")
        .bind(account_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Drops presence older than its lifetime.
pub async fn purge_presence(pool: &PgPool) -> sqlx::Result<u64> {
    Ok(
        sqlx::query("DELETE FROM presence WHERE updated_at <= now() - make_interval(secs => $1)")
            .bind(PRESENCE_TTL_SECONDS as f64)
            .execute(pool)
            .await?
            .rows_affected(),
    )
}

/// A friend and, while they are online, their latest presence.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Friend {
    pub name: String,
    pub presence: Option<Vec<u8>>,
    pub presence_at: Option<OffsetDateTime>,
}

impl Friend {
    pub fn online(&self) -> bool {
        self.presence.is_some()
    }
}

/// Everything one player's social lists hold. Names are canonical; lists are in name order,
/// requests oldest first.
#[derive(Debug, Clone)]
pub struct Overview {
    pub friends: Vec<Friend>,
    /// Requests waiting for this player, except from players they block.
    pub incoming: Vec<String>,
    pub outgoing: Vec<String>,
    pub blocked: Vec<String>,
}

pub async fn overview(pool: &PgPool, me: i32) -> sqlx::Result<Overview> {
    let friends = sqlx::query_as(
        "SELECT a.player_name AS name, p.data AS presence, p.updated_at AS presence_at
         FROM friendships f
         JOIN accounts a
           ON a.id = CASE WHEN f.account_lo = $1 THEN f.account_hi ELSE f.account_lo END
         LEFT JOIN presence p
           ON p.account_id = a.id AND p.updated_at > now() - make_interval(secs => $2)
         WHERE (f.account_lo = $1 OR f.account_hi = $1) AND a.player_name IS NOT NULL
         ORDER BY lower(a.player_name)",
    )
    .bind(me)
    .bind(PRESENCE_TTL_SECONDS as f64)
    .fetch_all(pool)
    .await?;
    let names = |sql: &'static str| async move {
        sqlx::query_as::<_, (String,)>(sql)
            .bind(me)
            .fetch_all(pool)
            .await
            .map(|rows| rows.into_iter().map(|(n,)| n).collect::<Vec<_>>())
    };
    Ok(Overview {
        friends,
        incoming: names(
            "SELECT a.player_name FROM friend_requests r JOIN accounts a ON a.id = r.from_account
             WHERE r.to_account = $1 AND a.player_name IS NOT NULL
               AND NOT EXISTS (SELECT 1 FROM blocks b
                               WHERE b.blocker = $1 AND b.blocked = r.from_account)
             ORDER BY r.created_at, lower(a.player_name)",
        )
        .await?,
        outgoing: names(
            "SELECT a.player_name FROM friend_requests r JOIN accounts a ON a.id = r.to_account
             WHERE r.from_account = $1 AND a.player_name IS NOT NULL
             ORDER BY r.created_at, lower(a.player_name)",
        )
        .await?,
        blocked: names(
            "SELECT a.player_name FROM blocks b JOIN accounts a ON a.id = b.blocked
             WHERE b.blocker = $1 AND a.player_name IS NOT NULL ORDER BY lower(a.player_name)",
        )
        .await?,
    })
}

/// Sizes of one account's social lists, for staff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::FromRow)]
pub struct Counts {
    pub friends: i64,
    pub incoming: i64,
    pub outgoing: i64,
    pub blocks: i64,
}

pub async fn counts(pool: &PgPool, account_id: i32) -> sqlx::Result<Counts> {
    sqlx::query_as(
        "SELECT
           (SELECT count(*) FROM friendships WHERE account_lo = $1 OR account_hi = $1) AS friends,
           (SELECT count(*) FROM friend_requests WHERE to_account = $1) AS incoming,
           (SELECT count(*) FROM friend_requests WHERE from_account = $1) AS outgoing,
           (SELECT count(*) FROM blocks WHERE blocker = $1) AS blocks",
    )
    .bind(account_id)
    .fetch_one(pool)
    .await
}
