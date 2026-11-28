//! Console identities are linked by possession of a verified-console proof and a portal session.
//!
//! Only Medius issues proofs, after verifying a Sony ticket. This module never issues a proof
//! from a display name or stores a raw code. All mutation paths lock the local account so
//! redemption serializes with account deletion, bans and other console changes.

use sqlx::PgPool;
use time::OffsetDateTime;

use crate::{names, tokens};

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Identity {
    pub provider: String,
    pub subject: String,
    pub online_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedeemOutcome {
    Linked,
    InvalidProof,
    IneligibleAccount,
    Conflict,
}

/// Hashes the ASCII lowercase 32-hex-character code string, not its decoded bytes.
/// Outer whitespace and uppercase ASCII are accepted; all other shapes are rejected.
fn code_hash(raw: &str) -> Option<[u8; 32]> {
    let code = raw.trim();
    if code.len() != 32 || !code.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(tokens::sha256(code.to_ascii_lowercase().as_bytes()))
}

/// Redeems once, proving both portal and console possession. A conflict rolls back the
/// deletion of the proof, so a code never disappears without creating its identity mapping.
pub async fn redeem(pool: &PgPool, account_id: i32, code: &str) -> sqlx::Result<RedeemOutcome> {
    let Some(hash) = code_hash(code) else {
        return Ok(RedeemOutcome::InvalidProof);
    };
    let mut tx = pool.begin().await?;
    let account: Option<(String, Option<String>, bool)> = sqlx::query_as(
        "SELECT kind, player_name, banned_at IS NOT NULL FROM accounts WHERE id = $1 FOR UPDATE",
    )
    .bind(account_id)
    .fetch_optional(&mut *tx)
    .await?;
    let eligible = account.is_some_and(|(kind, name, banned)| {
        kind == "person" && !banned && name.as_deref().is_some_and(names::valid_player_name)
    });
    if !eligible {
        return Ok(RedeemOutcome::IneligibleAccount);
    }
    // Lock the proof without an expiry predicate. PostgreSQL can evaluate a DELETE's
    // predicate before waiting on an unchanged locked row, so that predicate alone would
    // admit a proof which expires during the wait. Check current time in a separate SQL
    // statement only after both account and proof row locks have been acquired.
    let expires_at: Option<OffsetDateTime> = sqlx::query_scalar(
        "SELECT expires_at FROM console_link_proofs
         WHERE code_hash = $1 AND provider = 'psn' FOR UPDATE",
    )
    .bind(&hash[..])
    .fetch_optional(&mut *tx)
    .await?;
    let Some(expires_at) = expires_at else {
        return Ok(RedeemOutcome::InvalidProof);
    };
    let unexpired: bool = sqlx::query_scalar("SELECT $1::timestamptz > clock_timestamp()")
        .bind(expires_at)
        .fetch_one(&mut *tx)
        .await?;
    if !unexpired {
        return Ok(RedeemOutcome::InvalidProof);
    }
    let proof: Option<Identity> = sqlx::query_as(
        "DELETE FROM console_link_proofs
         WHERE code_hash = $1 AND provider = 'psn'
         RETURNING provider, subject, online_id",
    )
    .bind(&hash[..])
    .fetch_optional(&mut *tx)
    .await?;
    let Some(proof) = proof else {
        return Ok(RedeemOutcome::InvalidProof);
    };
    let inserted = sqlx::query(
        "INSERT INTO external_identities (provider, subject, account_id, online_id)
         VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
    )
    .bind(&proof.provider)
    .bind(&proof.subject)
    .bind(account_id)
    .bind(&proof.online_id)
    .execute(&mut *tx)
    .await?;
    if inserted.rows_affected() != 1 {
        tx.rollback().await?;
        return Ok(RedeemOutcome::Conflict);
    }
    tx.commit().await?;
    Ok(RedeemOutcome::Linked)
}

pub async fn list(pool: &PgPool, account_id: i32) -> sqlx::Result<Vec<Identity>> {
    sqlx::query_as(
        "SELECT provider, subject, online_id FROM external_identities
         WHERE account_id = $1 ORDER BY provider",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
}

/// Removes only the authenticated local account's mapping. Banned and unnamed people may
/// still remove a link; they cannot create one. Session ownership is enforced by the caller.
pub async fn unlink(pool: &PgPool, account_id: i32) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT id FROM accounts WHERE id = $1 FOR UPDATE")
        .bind(account_id)
        .fetch_optional(&mut *tx)
        .await?;
    let deleted =
        sqlx::query("DELETE FROM external_identities WHERE account_id = $1 AND provider = 'psn'")
            .bind(account_id)
            .execute(&mut *tx)
            .await?;
    tx.commit().await?;
    Ok(deleted.rows_affected() == 1)
}
