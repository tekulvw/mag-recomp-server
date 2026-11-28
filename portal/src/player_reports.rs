//! Player reports: signed-in players report another player's conduct, and moderators work
//! through the reports in the admin panel (docs/portal-player-reports.md).
//!
//! A report names a character by its game server key (the lowercase player name). Nothing is
//! attached automatically and nothing happens to the reported player until a moderator acts
//! with the existing admin tools. Reporters see only whether their report is still open or has
//! been reviewed, never the moderator's note or decision, and reported players see nothing.

use std::collections::{BTreeMap, HashMap};

use sqlx::PgPool;
use time::OffsetDateTime;

use crate::db::Account;
use crate::game_admin::{GameAdmin, valid_account};

/// Reports one account may file in 24 hours, unless `PORTAL_PLAYER_REPORTS_PER_DAY` says else.
pub const DEFAULT_PER_DAY: i64 = 5;
/// The longest description, in Unicode scalar values.
pub const MAX_DESCRIPTION: usize = 1000;
/// The longest moderator note.
pub const MAX_NOTE: usize = 1000;
/// Suggestions the report form offers at most.
pub const MAX_SUGGESTIONS: usize = 10;
/// The shortest text the form looks names up for.
pub const MIN_QUERY: usize = 2;

/// The advisory-lock key space of report filing, per reporter, so the daily allowance and the
/// one-open-report rule hold against concurrent submissions. Two-key locks never collide with
/// the one-key name locks (`db::lock_name`) or the social locks, which use another space.
const LOCK_SPACE: i32 = 0x5052_5054;

/// Why a player is reported. One per report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Cheating,
    Harassment,
    Griefing,
    OffensiveName,
    Other,
}

impl Reason {
    pub const ALL: [Reason; 5] = [
        Reason::Cheating,
        Reason::Harassment,
        Reason::Griefing,
        Reason::OffensiveName,
        Reason::Other,
    ];

    /// The stored value.
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::Cheating => "cheating",
            Reason::Harassment => "harassment",
            Reason::Griefing => "griefing",
            Reason::OffensiveName => "offensive_name",
            Reason::Other => "other",
        }
    }

    /// What the pages show.
    pub fn label(self) -> &'static str {
        match self {
            Reason::Cheating => "Cheating",
            Reason::Harassment => "Harassment or voice abuse",
            Reason::Griefing => "Team-killing or griefing",
            Reason::OffensiveName => "Offensive name",
            Reason::Other => "Other",
        }
    }

    pub fn parse(value: &str) -> Option<Reason> {
        Reason::ALL.into_iter().find(|r| r.as_str() == value)
    }

    /// The label for a stored value (an unknown one shows as stored).
    pub fn label_of(value: &str) -> &str {
        match Reason::parse(value) {
            Some(r) => r.label(),
            None => value,
        }
    }
}

/// A report's review state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Open,
    Actioned,
    Dismissed,
}

impl Status {
    pub const ALL: [Status; 3] = [Status::Open, Status::Actioned, Status::Dismissed];

    pub fn as_str(self) -> &'static str {
        match self {
            Status::Open => "open",
            Status::Actioned => "actioned",
            Status::Dismissed => "dismissed",
        }
    }

    pub fn parse(value: &str) -> Option<Status> {
        Status::ALL.into_iter().find(|s| s.as_str() == value)
    }

    /// What the reporter sees: open, or reviewed (never which way it went).
    pub fn reporter_view(value: &str) -> &'static str {
        if value == Status::Open.as_str() {
            "open"
        } else {
            "reviewed"
        }
    }
}

/// Free text from a form: line breaks made `\n`, other control characters (and direction
/// overrides, which could disguise the text) dropped, outer whitespace trimmed.
pub fn clean_text(raw: &str) -> String {
    let unified = raw.replace("\r\n", "\n").replace('\r', "\n");
    unified
        .chars()
        .filter(|c| *c == '\n' || *c == '\t' || !c.is_control())
        .filter(|c| !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
        .collect::<String>()
        .trim()
        .to_owned()
}

/// The game server key a typed name refers to, if it could be one.
pub fn target_key(raw: &str) -> Option<String> {
    let key = raw.trim().to_ascii_lowercase();
    valid_account(&key).then_some(key)
}

// ---- Who can be reported ----

/// The character a typed name refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub key: String,
    /// The best spelling known: the portal account's, the reserved name's, else the key.
    pub name: String,
    /// The portal account holding the name, if any.
    pub account: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    Found(Target),
    /// No account, reserved name or game server character has that name.
    Unknown,
    /// Only the game server could say, and it cannot be reached right now.
    Unavailable,
}

/// The portal account or reserved name with this key: `(spelling, account)`.
async fn portal_name(
    pool: &PgPool,
    key: &str,
) -> sqlx::Result<Option<(String, String, Option<i32>)>> {
    sqlx::query_as(
        "SELECT game_account_name, player_name, id FROM accounts WHERE lower(player_name) = $1 OR game_account_name=$1
         UNION ALL
         SELECT coalesce(game_account_name,name_key), name, owner_account_id FROM reserved_names WHERE name_key = $1
         LIMIT 1",
    )
    .bind(key)
    .fetch_optional(pool)
    .await
}

/// Finds the character a typed name refers to: a portal player name, a reserved name (an
/// existing player's, or a deleted account's), or a character the game server has.
pub async fn resolve(pool: &PgPool, game: Option<&GameAdmin>, raw: &str) -> sqlx::Result<Resolved> {
    let Some(key) = target_key(raw) else {
        return Ok(Resolved::Unknown);
    };
    if let Some((key, name, account)) = portal_name(pool, &key).await? {
        return Ok(Resolved::Found(Target { key, name, account }));
    }
    let Some(game) = game else {
        return Ok(Resolved::Unknown);
    };
    Ok(match game.character_keys().await {
        Ok(keys) if keys.binary_search(&key).is_ok() => Resolved::Found(Target {
            name: key.clone(),
            key,
            account: None,
        }),
        Ok(_) => Resolved::Unknown,
        Err(_) => Resolved::Unavailable,
    })
}

/// Up to [`MAX_SUGGESTIONS`] character names containing `query` (any case), names starting
/// with it first, without `exclude` (the reporter's own key). Names only: no account numbers
/// or emails. The game server's list is used when it is configured and reachable; portal
/// player names and reserved names always are.
pub async fn suggest(
    pool: &PgPool,
    game: Option<&GameAdmin>,
    query: &str,
    exclude: Option<&str>,
) -> sqlx::Result<Vec<String>> {
    let q = query.trim().to_ascii_lowercase();
    if q.chars().count() < MIN_QUERY || !valid_account(&q) {
        return Ok(Vec::new());
    }
    let pattern = format!("%{}%", crate::staff::like_literal(&q));
    let prefix = format!("{}%", crate::staff::like_literal(&q));
    let fetch = i64::try_from(MAX_SUGGESTIONS * 3).unwrap_or(30);
    let portal: Vec<(String, String)> = sqlx::query_as(
        "SELECT name_key, spelling FROM (
             SELECT game_account_name AS name_key, player_name AS spelling FROM accounts
             WHERE player_name IS NOT NULL AND lower(player_name) LIKE $1
             UNION
             SELECT coalesce(game_account_name,name_key), name FROM reserved_names WHERE name_key LIKE $1 AND owner_account_id IS NULL
         ) found
         ORDER BY name_key LIKE $2 DESC, name_key LIMIT $3",
    )
    .bind(&pattern)
    .bind(&prefix)
    .bind(fetch)
    .fetch_all(pool)
    .await?;
    // Sorted by (not a prefix match, key), so prefix matches come first.
    let mut found: BTreeMap<String, String> = BTreeMap::new();
    for (key, name) in portal {
        found.entry(key).or_insert(name);
    }
    if let Some(game) = game
        && let Ok(keys) = game.character_keys().await
    {
        let mut extra: Vec<String> = keys
            .iter()
            .filter(|k| k.contains(&q) && !found.contains_key(*k))
            .cloned()
            .collect();
        extra.sort_by_key(|k| (!k.starts_with(&q), k.clone()));
        extra.truncate(MAX_SUGGESTIONS * 3);
        // Spell them as their portal account or reserved name does, when one exists.
        let spelled: HashMap<String, String> = sqlx::query_as(
            "SELECT game_account_name, player_name FROM accounts WHERE game_account_name = ANY($1)
             UNION
             SELECT coalesce(game_account_name,name_key), name FROM reserved_names
             WHERE coalesce(game_account_name,name_key) = ANY($1) AND owner_account_id IS NULL",
        )
        .bind(&extra)
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect();
        for key in extra {
            let name = spelled.get(&key).cloned().unwrap_or_else(|| key.clone());
            found.entry(key).or_insert(name);
        }
    }
    let mut found: Vec<String> = found
        .into_iter()
        .filter(|(key, _)| Some(key.as_str()) != exclude)
        .map(|(_, name)| name)
        .collect();
    found.sort_by_key(|name| {
        (
            !name.to_ascii_lowercase().starts_with(&q),
            name.to_ascii_lowercase(),
        )
    });
    found.truncate(MAX_SUGGESTIONS);
    Ok(found)
}

// ---- Filing ----

pub struct NewReport<'a> {
    pub reporter: i32,
    pub target: &'a Target,
    pub reason: Reason,
    pub description: &'a str,
    pub occurred_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitOutcome {
    Filed(i64),
    /// The reporter already has an open report about this character.
    Duplicate,
    /// The reporter has filed their allowance in the last 24 hours.
    RateLimited,
}

/// Files a report, unless the reporter already has an open one about the same character or
/// has used up `per_day` in the last 24 hours. Neither refusal creates a row or counts.
pub async fn submit(
    pool: &PgPool,
    new: &NewReport<'_>,
    per_day: i64,
) -> sqlx::Result<SubmitOutcome> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock($1, $2)")
        .bind(LOCK_SPACE)
        .bind(new.reporter)
        .execute(&mut *tx)
        .await?;
    let (open, recent): (bool, i64) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM player_reports
                        WHERE reporter_account = $1 AND target_key = $2 AND status = 'open'),
                (SELECT count(*) FROM player_reports
                 WHERE reporter_account = $1 AND created_at > now() - interval '24 hours')",
    )
    .bind(new.reporter)
    .bind(&new.target.key)
    .fetch_one(&mut *tx)
    .await?;
    if open {
        return Ok(SubmitOutcome::Duplicate);
    }
    if recent >= per_day {
        return Ok(SubmitOutcome::RateLimited);
    }
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO player_reports
             (reporter_account, target_key, target_name, target_account, reason, description, occurred_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id",
    )
    .bind(new.reporter)
    .bind(&new.target.key)
    .bind(&new.target.name)
    .bind(new.target.account)
    .bind(new.reason.as_str())
    .bind(new.description)
    .bind(new.occurred_at)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(SubmitOutcome::Filed(id))
}

/// One of the reporter's own reports, as "My reports" shows it: no review details.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MyReport {
    pub id: i64,
    pub target_name: String,
    pub reason: String,
    pub created_at: OffsetDateTime,
    pub status: String,
}

pub async fn mine(pool: &PgPool, reporter: i32, limit: i64) -> sqlx::Result<Vec<MyReport>> {
    sqlx::query_as(
        "SELECT id, target_name, reason, created_at, status FROM player_reports
         WHERE reporter_account = $1 ORDER BY created_at DESC, id DESC LIMIT $2",
    )
    .bind(reporter)
    .bind(limit)
    .fetch_all(pool)
    .await
}

// ---- The moderators' queue ----

/// A report as moderators see it, with the reporter's current player name and how many other
/// reports name the same character.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Report {
    pub id: i64,
    pub reporter_account: Option<i32>,
    pub reporter_name: Option<String>,
    pub target_key: String,
    pub target_name: String,
    pub target_account: Option<i32>,
    pub reason: String,
    pub description: String,
    pub occurred_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub status: String,
    pub reviewed_by_email: Option<String>,
    pub review_note: Option<String>,
    pub reviewed_at: Option<OffsetDateTime>,
    pub other_reports: i64,
    pub other_open: i64,
}

/// The report columns of [`Report`], followed by literal SQL.
macro_rules! report_sql {
    ($tail:literal) => {
        concat!(
            "SELECT r.id, r.reporter_account, a.player_name AS reporter_name,
        r.target_key, r.target_name, r.target_account, r.reason, r.description, r.occurred_at,
        r.created_at, r.status, r.reviewed_by_email, r.review_note, r.reviewed_at,
        (SELECT count(*) FROM player_reports o WHERE o.target_key = r.target_key AND o.id <> r.id)
            AS other_reports,
        (SELECT count(*) FROM player_reports o
         WHERE o.target_key = r.target_key AND o.id <> r.id AND o.status = 'open') AS other_open
     FROM player_reports r LEFT JOIN accounts a ON a.id = r.reporter_account ",
            $tail
        )
    };
}

/// Reports with this status and reason (`None`: any), newest first.
pub async fn queue(
    pool: &PgPool,
    status: Option<Status>,
    reason: Option<Reason>,
    limit: i64,
) -> sqlx::Result<Vec<Report>> {
    sqlx::query_as(report_sql!(
        "WHERE ($1::text IS NULL OR r.status = $1) AND ($2::text IS NULL OR r.reason = $2)
         ORDER BY r.created_at DESC, r.id DESC LIMIT $3"
    ))
    .bind(status.map(Status::as_str))
    .bind(reason.map(Reason::as_str))
    .bind(limit)
    .fetch_all(pool)
    .await
}

pub async fn report(pool: &PgPool, id: i64) -> sqlx::Result<Option<Report>> {
    sqlx::query_as(report_sql!("WHERE r.id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// The other reports about a character, newest first.
pub async fn about(
    pool: &PgPool,
    target_key: &str,
    except: i64,
    limit: i64,
) -> sqlx::Result<Vec<Report>> {
    sqlx::query_as(report_sql!(
        "WHERE r.target_key = $1 AND r.id <> $2
         ORDER BY r.created_at DESC, r.id DESC LIMIT $3"
    ))
    .bind(target_key)
    .bind(except)
    .bind(limit)
    .fetch_all(pool)
    .await
}

pub async fn open_count(pool: &PgPool) -> sqlx::Result<i64> {
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM player_reports WHERE status = 'open'")
        .fetch_one(pool)
        .await?;
    Ok(n)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewOutcome {
    Changed,
    NoSuchReport,
    /// The report already has that status.
    Unchanged,
    /// Reopening would give the reporter two open reports about the same character.
    ReporterHasAnotherOpen,
}

/// Moves a report to `to` with the moderator's note, and writes the change to the audit log in
/// the same transaction.
pub async fn review(
    pool: &PgPool,
    id: i64,
    to: Status,
    note: &str,
    actor: &Account,
) -> sqlx::Result<ReviewOutcome> {
    let mut tx = pool.begin().await?;
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT status, target_name FROM player_reports WHERE id = $1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((from, target_name)) = row else {
        return Ok(ReviewOutcome::NoSuchReport);
    };
    if from == to.as_str() {
        return Ok(ReviewOutcome::Unchanged);
    }
    let note = (!note.is_empty()).then_some(note);
    let updated = sqlx::query(
        "UPDATE player_reports SET status = $2, reviewed_by = $3, reviewed_by_email = $4,
                review_note = $5, reviewed_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(to.as_str())
    .bind(actor.id)
    .bind(actor.email.as_deref().unwrap_or(""))
    .bind(note)
    .execute(&mut *tx)
    .await;
    match updated {
        Ok(_) => {}
        Err(sqlx::Error::Database(d)) if d.code().as_deref() == Some("23505") => {
            return Ok(ReviewOutcome::ReporterHasAnotherOpen);
        }
        Err(e) => return Err(e),
    }
    let action = match to {
        Status::Open => "reopen player report",
        Status::Actioned => "action player report",
        Status::Dismissed => "dismiss player report",
    };
    crate::staff::log(
        &mut *tx,
        actor,
        action,
        &format!("report {id} on {target_name}"),
        note.unwrap_or(""),
    )
    .await?;
    tx.commit().await?;
    Ok(ReviewOutcome::Changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasons_and_statuses() {
        for r in Reason::ALL {
            assert_eq!(Reason::parse(r.as_str()), Some(r));
        }
        assert_eq!(Reason::parse("Cheating"), None);
        assert_eq!(Reason::label_of("griefing"), "Team-killing or griefing");
        assert_eq!(Reason::label_of("mystery"), "mystery");
        for s in Status::ALL {
            assert_eq!(Status::parse(s.as_str()), Some(s));
        }
        assert_eq!(Status::reporter_view("open"), "open");
        assert_eq!(Status::reporter_view("actioned"), "reviewed");
        assert_eq!(Status::reporter_view("dismissed"), "reviewed");
    }

    #[test]
    fn text_is_cleaned() {
        assert_eq!(clean_text("  a\r\nb\rc\u{7}\u{202e}d\t \n"), "a\nb\ncd");
        assert_eq!(clean_text(" \n "), "");
        assert_eq!(target_key(" Alpha_7 ").as_deref(), Some("alpha_7"));
        for bad in ["", "a b", "näme", &"a".repeat(33)] {
            assert_eq!(target_key(bad), None, "{bad}");
        }
    }
}
