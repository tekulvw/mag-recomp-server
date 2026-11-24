//! Performance and error reports from launchers (`POST /api/v1/reports`,
//! docs/portal-reports.md): checking a report, storing it, the per-account limits, retention,
//! and the queries the admin CLI and panel use.
//!
//! Metadata and the small JSON documents go to the `reports` table; gzip'd logs go to the
//! report store (`report_store`), written before the row so a row never points at missing
//! logs. No client address is kept.

use std::collections::BTreeMap;
use std::io::Read;
use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;
use serde_json::Value;
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};

use crate::db::Account;
use crate::report_store::{self, ReportStore};

pub const SCHEMA_VERSION: u64 = 1;
/// The largest request body the endpoint reads.
pub const MAX_BODY: usize = 4 << 20;
pub const MAX_CLIENT: usize = 16 << 10;
/// `session` holds a player comment of up to `MAX_COMMENT` characters, which serializes to at
/// most 6002 bytes (each character escaped as `\u00XX`), plus 4 KiB or more for everything else.
pub const MAX_SESSION: usize = 12 << 10;
/// The longest `session.user_comment`, in Unicode scalar values.
pub const MAX_COMMENT: usize = 1000;
/// How many characters of a comment the report lists show.
pub const COMMENT_PREVIEW: usize = 60;
pub const MAX_METRICS: usize = 64 << 10;
pub const MAX_LOGS: usize = 8;
pub const MAX_LOG: usize = 2 << 20;
pub const MAX_LOGS_TOTAL: usize = 3 << 20;
/// Rolling 24-hour report allowance per account.
pub const PERSON_DAILY: i64 = 20;
pub const SERVICE_DAILY: i64 = 200;
/// The most a log is inflated to for staff (CLI and panel).
pub const MAX_DECOMPRESSED: u64 = 64 << 20;

pub const TRIGGERS: [&str; 6] = [
    "crash",
    "launch_error",
    "halt",
    "slow_frames",
    "audio",
    "manual",
];
pub const PLATFORMS: [&str; 3] = ["macos", "linux", "windows"];

/// The report service: where logs go and how long reports stay.
pub struct Reports {
    pub store: Arc<dyn ReportStore>,
    pub retention_days: i64,
    pub person_daily: i64,
    pub service_daily: i64,
}

impl Reports {
    pub fn new(store: Arc<dyn ReportStore>, retention_days: i64) -> Self {
        Self {
            store,
            retention_days,
            person_daily: PERSON_DAILY,
            service_daily: SERVICE_DAILY,
        }
    }

    fn daily_limit(&self, account: &Account) -> i64 {
        if account.is_service() {
            self.service_daily
        } else {
            self.person_daily
        }
    }
}

/// The report ID in canonical lowercase form, if `raw` is a version 4 UUID.
pub fn valid_report_id(raw: &str) -> Option<String> {
    let b = raw.as_bytes();
    let shape = b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        });
    // Version nibble 4, RFC 4122 variant (8, 9, a or b).
    (shape && b[14] == b'4' && matches!(b[19].to_ascii_lowercase(), b'8' | b'9' | b'a' | b'b'))
        .then(|| raw.to_ascii_lowercase())
}

/// A log name: `^[a-z0-9][a-z0-9._-]{0,63}$`.
pub fn valid_log_name(name: &str) -> bool {
    let b = name.as_bytes();
    (1..=64).contains(&b.len())
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"._-".contains(c))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawReport {
    schema_version: u64,
    report_id: String,
    trigger: String,
    client: Value,
    #[serde(default)]
    session: Option<Value>,
    #[serde(default)]
    metrics: Option<Value>,
    #[serde(default)]
    logs: Option<BTreeMap<String, RawLog>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLog {
    encoding: String,
    data: String,
}

/// A report that passed every check, with its logs decoded (still gzip'd).
#[derive(Debug)]
pub struct NewReport {
    pub id: String,
    pub trigger: String,
    pub platform: String,
    pub client_version: String,
    pub client: String,
    pub session: Option<String>,
    pub metrics: Option<String>,
    pub logs: Vec<(String, Vec<u8>)>,
}

impl NewReport {
    pub fn log_bytes(&self) -> i64 {
        self.logs.iter().map(|(_, d)| d.len() as i64).sum()
    }
}

/// Why a report was refused, as the short `detail` of a 400.
#[derive(Debug, PartialEq, Eq)]
pub struct Invalid(pub String);

fn invalid(detail: impl Into<String>) -> Invalid {
    Invalid(detail.into())
}

/// A free-form JSON object, serialized, if it fits in `max` bytes.
fn object(field: &str, value: Value, max: usize) -> Result<String, Invalid> {
    if !value.is_object() {
        return Err(invalid(format!("{field} must be an object")));
    }
    let text =
        serde_json::to_string(&value).map_err(|_| invalid(format!("{field} is not JSON")))?;
    if text.len() > max {
        return Err(invalid(format!("{field} is larger than {max} bytes")));
    }
    Ok(text)
}

fn optional_object(
    field: &str,
    value: Option<Value>,
    max: usize,
) -> Result<Option<String>, Invalid> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(v) => object(field, v, max).map(Some),
    }
}

/// Checks a request body against the contract (docs/portal-reports.md).
pub fn parse(body: &[u8]) -> Result<NewReport, Invalid> {
    let raw: RawReport = serde_json::from_slice(body).map_err(|e| {
        // serde's message names the field or position, never echoes the body.
        let mut detail = e.to_string();
        detail.truncate(160);
        invalid(detail)
    })?;
    if raw.schema_version != SCHEMA_VERSION {
        return Err(invalid("unsupported schema_version"));
    }
    let id =
        valid_report_id(&raw.report_id).ok_or_else(|| invalid("report_id must be a UUID v4"))?;
    if !TRIGGERS.contains(&raw.trigger.as_str()) {
        return Err(invalid("unknown trigger"));
    }
    let client_version = match raw.client.get("version") {
        Some(Value::String(v)) if (1..=64).contains(&v.chars().count()) => v.clone(),
        _ => {
            return Err(invalid(
                "client.version must be a string of 1 to 64 characters",
            ));
        }
    };
    let platform = match raw.client.get("platform") {
        Some(Value::String(p)) if PLATFORMS.contains(&p.as_str()) => p.clone(),
        _ => return Err(invalid("client.platform must be macos, linux or windows")),
    };
    let client = object("client", raw.client, MAX_CLIENT)?;
    // Checked before the size of `session`, so an overlong comment gets its own answer. Any
    // other kind of value is free-form JSON like the rest of `session`.
    if let Some(Value::String(c)) = raw.session.as_ref().and_then(|s| s.get("user_comment"))
        && c.chars().count() > MAX_COMMENT
    {
        return Err(invalid("user_comment too long"));
    }
    let session = optional_object("session", raw.session, MAX_SESSION)?;
    let metrics = optional_object("metrics", raw.metrics, MAX_METRICS)?;
    let raw_logs = raw.logs.unwrap_or_default();
    if raw_logs.len() > MAX_LOGS {
        return Err(invalid(format!("at most {MAX_LOGS} logs")));
    }
    let mut logs = Vec::with_capacity(raw_logs.len());
    let mut total = 0usize;
    for (name, log) in raw_logs {
        if !valid_log_name(&name) {
            return Err(invalid("log names must match ^[a-z0-9][a-z0-9._-]{0,63}$"));
        }
        if log.encoding != "gzip+base64" {
            return Err(invalid(format!("log {name}: encoding must be gzip+base64")));
        }
        // Base64 grows data by 4/3; refuse oversized logs before decoding them.
        if log.data.len() > MAX_LOG.div_ceil(3) * 4 {
            return Err(invalid(format!(
                "log {name} is larger than {MAX_LOG} bytes"
            )));
        }
        let data = STANDARD
            .decode(log.data.as_bytes())
            .map_err(|_| invalid(format!("log {name} is not base64")))?;
        if data.len() > MAX_LOG {
            return Err(invalid(format!(
                "log {name} is larger than {MAX_LOG} bytes"
            )));
        }
        if !data.starts_with(&[0x1f, 0x8b]) {
            return Err(invalid(format!("log {name} is not gzip")));
        }
        total += data.len();
        if total > MAX_LOGS_TOTAL {
            return Err(invalid(format!(
                "logs total more than {MAX_LOGS_TOTAL} bytes"
            )));
        }
        logs.push((name, data));
    }
    Ok(NewReport {
        id,
        trigger: raw.trigger,
        platform,
        client_version,
        client,
        session,
        metrics,
        logs,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum SubmitOutcome {
    /// Stored now.
    Created(OffsetDateTime),
    /// This account sent this report before; nothing new was stored.
    Duplicate(OffsetDateTime),
    /// Another account holds this report ID.
    Conflict,
    /// Over the account's daily allowance; retry after this many seconds.
    RateLimited(u64),
}

fn now_micros() -> OffsetDateTime {
    // PostgreSQL keeps microseconds; match it so the key's date and the row agree exactly.
    let now = OffsetDateTime::now_utc();
    now.replace_nanosecond(now.nanosecond() / 1000 * 1000)
        .unwrap_or(now)
}

/// Stores a checked report for `account`: logs first, then the row. Holds advisory locks on
/// the account and the report ID until it finishes, so a resend racing the original, or two
/// reports racing the allowance, are serialized.
pub async fn submit(
    pool: &PgPool,
    reports: &Reports,
    account: &Account,
    report: NewReport,
) -> anyhow::Result<SubmitOutcome> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('report-account:' || $1::text))")
        .bind(account.id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('report-id:' || $1))")
        .bind(&report.id)
        .execute(&mut *tx)
        .await?;
    let existing: Option<(i32, OffsetDateTime)> =
        sqlx::query_as("SELECT account_id, received_at FROM reports WHERE id = $1::uuid")
            .bind(&report.id)
            .fetch_optional(&mut *tx)
            .await?;
    match existing {
        Some((owner, at)) if owner == account.id => return Ok(SubmitOutcome::Duplicate(at)),
        Some(_) => return Ok(SubmitOutcome::Conflict),
        None => {}
    }
    let limit = reports.daily_limit(account);
    // The report that must age out of the window before another fits, if the window is full.
    let blocking: Option<(OffsetDateTime,)> = sqlx::query_as(
        "SELECT received_at FROM reports
         WHERE account_id = $1 AND received_at > now() - interval '24 hours'
         ORDER BY received_at DESC OFFSET $2 LIMIT 1",
    )
    .bind(account.id)
    .bind(limit - 1)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((at,)) = blocking {
        let wait = (at + Duration::hours(24) - OffsetDateTime::now_utc()).whole_seconds();
        return Ok(SubmitOutcome::RateLimited(wait.max(1) as u64));
    }
    let received_at = now_micros();
    let mut written: Vec<String> = Vec::new();
    let mut failure: Option<anyhow::Error> = None;
    for (name, data) in &report.logs {
        let key = report_store::log_key(received_at, &report.id, name);
        match reports.store.put(&key, data.clone()).await {
            Ok(()) => written.push(key),
            Err(e) => {
                failure = Some(e);
                break;
            }
        }
    }
    if failure.is_none() {
        let names: Vec<&str> = report.logs.iter().map(|(n, _)| n.as_str()).collect();
        let inserted = sqlx::query(
            "INSERT INTO reports (id, account_id, received_at, trigger, platform, client_version,
                                  client, session, metrics, log_names, log_bytes)
             VALUES ($1::uuid, $2, $3, $4, $5, $6, $7::jsonb, $8::jsonb, $9::jsonb, $10, $11)",
        )
        .bind(&report.id)
        .bind(account.id)
        .bind(received_at)
        .bind(&report.trigger)
        .bind(&report.platform)
        .bind(&report.client_version)
        .bind(&report.client)
        .bind(&report.session)
        .bind(&report.metrics)
        .bind(&names)
        .bind(report.log_bytes())
        .execute(&mut *tx)
        .await;
        match inserted {
            Ok(_) => match tx.commit().await {
                Ok(()) => return Ok(SubmitOutcome::Created(received_at)),
                Err(e) => failure = Some(e.into()),
            },
            Err(e) => failure = Some(e.into()),
        }
    }
    // Nothing refers to the logs written so far; the locks are still held, so no other
    // request can have written the same keys.
    for key in &written {
        if let Err(e) = reports.store.delete(key).await {
            tracing::warn!(error = %e, "could not remove the logs of a failed report");
        }
    }
    Err(failure.expect("set on every path here"))
}

// ---- Retention ----

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Purged {
    pub reports: u64,
    pub orphans: u64,
}

/// Removes reports (rows and logs) received more than `retention_days` ago, then logs no row
/// refers to.
pub async fn purge(pool: &PgPool, reports: &Reports) -> anyhow::Result<Purged> {
    let cutoff = OffsetDateTime::now_utc() - Duration::days(reports.retention_days);
    let mut purged = Purged::default();
    loop {
        let batch: Vec<(String, OffsetDateTime, Vec<String>)> = sqlx::query_as(
            "SELECT id::text, received_at, log_names FROM reports WHERE received_at < $1
             ORDER BY received_at LIMIT 500",
        )
        .bind(cutoff)
        .fetch_all(pool)
        .await?;
        if batch.is_empty() {
            break;
        }
        let mut ids = Vec::with_capacity(batch.len());
        for (id, at, names) in batch {
            for name in names {
                // A log left behind is swept as an orphan, or expires in the bucket.
                if let Err(e) = reports
                    .store
                    .delete(&report_store::log_key(at, &id, &name))
                    .await
                {
                    tracing::warn!(error = %e, report = %id, "could not remove an expired log");
                }
            }
            ids.push(id);
        }
        purged.reports += sqlx::query("DELETE FROM reports WHERE id = ANY($1::uuid[])")
            .bind(&ids)
            .execute(pool)
            .await?
            .rows_affected();
    }
    purged.orphans = reports.store.sweep_orphans(pool, cutoff).await?;
    Ok(purged)
}

/// Purges every hour, for as long as the server runs.
pub fn spawn_purge(pool: PgPool, reports: Arc<Reports>) {
    tokio::spawn(async move {
        let mut every = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            every.tick().await;
            match purge(&pool, &reports).await {
                Ok(p) if p.reports > 0 || p.orphans > 0 => tracing::info!(
                    reports = p.reports,
                    orphans = p.orphans,
                    "expired reports removed"
                ),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "report purge failed"),
            }
        }
    });
}

// ---- Staff queries ----

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ReportRow {
    pub id: String,
    pub account_id: i32,
    pub player_name: Option<String>,
    pub account_kind: String,
    pub received_at: OffsetDateTime,
    pub trigger: String,
    pub platform: String,
    pub client_version: String,
    pub log_names: Vec<String>,
    pub log_bytes: i64,
    /// `session.user_comment`, when it is a string with something besides white space in it.
    pub user_comment: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ReportDetail {
    #[sqlx(flatten)]
    pub row: ReportRow,
    pub client: String,
    pub session: Option<String>,
    pub metrics: Option<String>,
}

/// `session.user_comment` if it is a string holding more than white space, else NULL.
macro_rules! comment_sql {
    () => {
        "CASE WHEN jsonb_typeof(r.session->'user_comment') = 'string'
                   AND r.session->>'user_comment' ~ '[^[:space:]]'
              THEN r.session->>'user_comment' END"
    };
}

macro_rules! report_sql {
    ($extra:literal, $tail:literal) => {
        concat!(
            "SELECT r.id::text AS id, r.account_id, a.player_name, a.kind AS account_kind,
                    r.received_at, r.trigger, r.platform, r.client_version, r.log_names,
                    r.log_bytes, ",
            comment_sql!(),
            " AS user_comment",
            $extra,
            " FROM reports r JOIN accounts a ON a.id = r.account_id ",
            $tail
        )
    };
}

#[derive(Debug, Clone, Default)]
pub struct Filter {
    /// A player name (any case).
    pub account: Option<String>,
    pub trigger: Option<String>,
    pub platform: Option<String>,
    pub since: Option<OffsetDateTime>,
    /// Only reports with a player comment.
    pub has_comment: bool,
}

/// Reports matching `filter`, newest first.
pub async fn list(pool: &PgPool, filter: &Filter, limit: i64) -> sqlx::Result<Vec<ReportRow>> {
    sqlx::query_as(concat!(
        report_sql!(
            "",
            "WHERE ($1::text IS NULL OR lower(a.player_name) = $1)
               AND ($2::text IS NULL OR r.trigger = $2)
               AND ($3::text IS NULL OR r.platform = $3)
               AND ($4::timestamptz IS NULL OR r.received_at >= $4)
               AND (NOT $5 OR "
        ),
        comment_sql!(),
        " IS NOT NULL) ORDER BY r.received_at DESC LIMIT $6"
    ))
    .bind(filter.account.as_deref().map(crate::names::name_key))
    .bind(&filter.trigger)
    .bind(&filter.platform)
    .bind(filter.since)
    .bind(filter.has_comment)
    .bind(limit)
    .fetch_all(pool)
    .await
}

pub async fn detail(pool: &PgPool, id: &str) -> sqlx::Result<Option<ReportDetail>> {
    let Some(id) = valid_report_id(id) else {
        return Ok(None);
    };
    sqlx::query_as(report_sql!(
        ", r.client::text AS client, r.session::text AS session, r.metrics::text AS metrics",
        "WHERE r.id = $1::uuid"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// Characters a comment never shows to staff: control characters (except as noted by the
/// callers) and the invisible marks that reorder text.
fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// A comment on one line for report lists: white space runs become one space, hidden
/// characters go, and past `max` characters it is cut and ends in an ellipsis.
pub fn comment_preview(text: &str, max: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| !hidden(*c))
        .collect();
    let line = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= max {
        return line;
    }
    let mut cut: String = line.chars().take(max).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    cut
}

/// A whole comment for staff to read: line breaks and tabs stay (`\r\n` and `\r` become `\n`),
/// other hidden characters go.
pub fn comment_text(text: &str) -> String {
    text.replace("\r\n", "\n")
        .chars()
        .map(|c| if c == '\r' { '\n' } else { c })
        .filter(|c| matches!(c, '\n' | '\t') || !hidden(*c))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct DayCount {
    pub day: String,
    pub trigger: String,
    pub platform: String,
    pub count: i64,
}

/// Reports per UTC day, trigger and platform over the last `days` days, newest day first.
pub async fn stats(pool: &PgPool, days: i64) -> sqlx::Result<Vec<DayCount>> {
    sqlx::query_as(
        "SELECT to_char(received_at AT TIME ZONE 'UTC', 'YYYY-MM-DD') AS day, trigger, platform,
                count(*) AS count
         FROM reports WHERE received_at >= date_trunc('day', now() AT TIME ZONE 'UTC')
                                            AT TIME ZONE 'UTC' - make_interval(days => $1)
         GROUP BY 1, 2, 3 ORDER BY 1 DESC, 2, 3",
    )
    .bind(i32::try_from(days.saturating_sub(1).max(0)).unwrap_or(i32::MAX))
    .fetch_all(pool)
    .await
}

/// A report's stored log, if the report lists it.
pub async fn log(
    store: &dyn ReportStore,
    report: &ReportRow,
    name: &str,
) -> anyhow::Result<Option<Vec<u8>>> {
    if !report.log_names.iter().any(|n| n == name) {
        return Ok(None);
    }
    store
        .get(&report_store::log_key(report.received_at, &report.id, name))
        .await
}

/// Inflates a gzip'd log (every member), refusing more than `max` bytes of output.
pub fn decompress(gz: &[u8], max: u64) -> anyhow::Result<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::MultiGzDecoder::new(gz)
        .take(max + 1)
        .read_to_end(&mut out)?;
    if out.len() as u64 > max {
        anyhow::bail!("the log inflates to more than {max} bytes");
    }
    Ok(out)
}

/// Removes the logs of every report of an account that is about to be deleted (its rows go
/// with the account). Best effort: a log left behind is swept as an orphan or expires.
pub async fn delete_account_logs(pool: &PgPool, store: &dyn ReportStore, account_id: i32) {
    let rows: Vec<(String, OffsetDateTime, Vec<String>)> = match sqlx::query_as(
        "SELECT id::text, received_at, log_names FROM reports WHERE account_id = $1",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, account = account_id, "could not list an account's reports");
            return;
        }
    };
    for (id, at, names) in rows {
        for name in names {
            if let Err(e) = store.delete(&report_store::log_key(at, &id, &name)).await {
                tracing::warn!(error = %e, report = %id, "could not remove a deleted account's log");
            }
        }
    }
}

/// Deletes a report's row, then its logs. False when there is no such report.
pub async fn delete(pool: &PgPool, store: &dyn ReportStore, id: &str) -> anyhow::Result<bool> {
    let Some(id) = valid_report_id(id) else {
        return Ok(false);
    };
    let row: Option<(OffsetDateTime, Vec<String>)> =
        sqlx::query_as("DELETE FROM reports WHERE id = $1::uuid RETURNING received_at, log_names")
            .bind(&id)
            .fetch_optional(pool)
            .await?;
    let Some((at, names)) = row else {
        return Ok(false);
    };
    for name in names {
        store.delete(&report_store::log_key(at, &id, &name)).await?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ID: &str = "0B9C4F5E-1D2A-4B3C-8D4E-5F6A7B8C9D0E";

    fn gz_b64() -> String {
        STANDARD.encode([0x1f, 0x8b, 8, 0, 0, 0, 0, 0])
    }

    fn base() -> Value {
        json!({
            "schema_version": 1,
            "report_id": ID,
            "trigger": "slow_frames",
            "client": { "version": "1.2.3", "platform": "linux", "gpu": "x" },
            "metrics": { "p99_ms": 41.5 },
            "logs": { "launcher.log": { "encoding": "gzip+base64", "data": gz_b64() } },
        })
    }

    fn check(v: &Value) -> Result<NewReport, Invalid> {
        parse(&serde_json::to_vec(v).unwrap())
    }

    #[test]
    fn ids_and_names() {
        assert_eq!(valid_report_id(ID).unwrap(), ID.to_ascii_lowercase());
        for bad in [
            "",
            "0b9c4f5e-1d2a-1b3c-8d4e-5f6a7b8c9d0e",
            "0b9c4f5e-1d2a-4b3c-cd4e-5f6a7b8c9d0e",
            "0b9c4f5e1d2a4b3c8d4e5f6a7b8c9d0e",
            "0b9c4f5e-1d2a-4b3c-8d4e-5f6a7b8c9d0g",
        ] {
            assert!(valid_report_id(bad).is_none(), "{bad}");
        }
        for good in ["a", "launcher.log", "0", "spu-trace_2.txt", &"a".repeat(64)] {
            assert!(valid_log_name(good), "{good}");
        }
        for bad in ["", ".x", "-x", "A", "a/b", "a b", &"a".repeat(65)] {
            assert!(!valid_log_name(bad), "{bad}");
        }
    }

    #[test]
    fn accepts_the_contract() {
        let r = check(&base()).unwrap();
        assert_eq!(r.id, ID.to_ascii_lowercase());
        assert_eq!(
            (r.trigger.as_str(), r.platform.as_str()),
            ("slow_frames", "linux")
        );
        assert_eq!(r.client_version, "1.2.3");
        assert_eq!(r.session, None);
        assert_eq!(r.logs.len(), 1);
        let mut minimal = base();
        let m = minimal.as_object_mut().unwrap();
        m.remove("metrics");
        m.remove("logs");
        m.insert("session".into(), Value::Null);
        assert!(check(&minimal).unwrap().logs.is_empty());
    }

    #[test]
    fn refuses_what_breaks_it() {
        type Change = Box<dyn Fn(&mut Value)>;
        let cases: Vec<(&str, Change)> = vec![
            ("unknown field", Box::new(|v| v["extra"] = json!(1))),
            ("schema", Box::new(|v| v["schema_version"] = json!(2))),
            ("id", Box::new(|v| v["report_id"] = json!("nope"))),
            ("trigger", Box::new(|v| v["trigger"] = json!("boom"))),
            ("client array", Box::new(|v| v["client"] = json!([]))),
            (
                "no version",
                Box::new(|v| v["client"]["version"] = json!("")),
            ),
            (
                "long version",
                Box::new(|v| v["client"]["version"] = json!("x".repeat(65))),
            ),
            (
                "platform",
                Box::new(|v| v["client"]["platform"] = json!("ps3")),
            ),
            (
                "big client",
                Box::new(|v| v["client"]["pad"] = json!("x".repeat(MAX_CLIENT))),
            ),
            ("session string", Box::new(|v| v["session"] = json!("x"))),
            (
                "big session",
                Box::new(|v| v["session"] = json!({"p": "x".repeat(MAX_SESSION)})),
            ),
            (
                "big metrics",
                Box::new(|v| v["metrics"] = json!({"p": "x".repeat(MAX_METRICS)})),
            ),
            (
                "log name",
                Box::new(|v| {
                    v["logs"] = json!({"Bad": {"encoding": "gzip+base64", "data": gz_b64()}})
                }),
            ),
            (
                "encoding",
                Box::new(|v| v["logs"]["launcher.log"]["encoding"] = json!("gzip")),
            ),
            (
                "not base64",
                Box::new(|v| v["logs"]["launcher.log"]["data"] = json!("!!!")),
            ),
            (
                "not gzip",
                Box::new(|v| v["logs"]["launcher.log"]["data"] = json!(STANDARD.encode(b"plain"))),
            ),
            (
                "log field",
                Box::new(|v| v["logs"]["launcher.log"]["x"] = json!(1)),
            ),
            (
                "too many",
                Box::new(|v| {
                    let logs: serde_json::Map<String, Value> = (0..9)
                        .map(|i| {
                            (
                                format!("l{i}"),
                                json!({"encoding": "gzip+base64", "data": gz_b64()}),
                            )
                        })
                        .collect();
                    v["logs"] = Value::Object(logs);
                }),
            ),
        ];
        for (what, change) in cases {
            let mut v = base();
            change(&mut v);
            assert!(check(&v).is_err(), "{what}");
        }
        assert!(parse(b"{").is_err());
    }

    #[test]
    fn comment_limits() {
        let with = |comment: Value| {
            let mut v = base();
            v["trigger"] = json!("manual");
            v["session"] = json!({ "server": "play.example.test", "user_comment": comment });
            check(&v)
        };
        // At the limit, in the characters that serialize longest, the session still fits.
        for c in ["x", "\u{4e2d}", "\u{1f600}", "\"", "\u{1}"] {
            let r = with(json!(c.repeat(MAX_COMMENT))).unwrap();
            let session: Value = serde_json::from_str(r.session.as_deref().unwrap()).unwrap();
            assert_eq!(session["user_comment"], json!(c.repeat(MAX_COMMENT)));
            assert_eq!(
                with(json!(c.repeat(MAX_COMMENT + 1))).unwrap_err(),
                Invalid("user_comment too long".into())
            );
        }
        // Other kinds of value are free-form, like the rest of the session.
        assert!(with(json!(42)).is_ok());
        assert!(with(json!({"text": "x".repeat(2 * MAX_COMMENT)})).is_ok());
        assert!(
            with(json!({"text": "x".repeat(MAX_SESSION)}))
                .unwrap_err()
                .0
                .contains("larger")
        );
    }

    #[test]
    fn comment_display() {
        assert_eq!(comment_preview("  short  ", 60), "short");
        assert_eq!(
            comment_preview("one\ntwo\r\n\tthree\u{1b}[31m red\u{202e}x\u{7}", 60),
            "one two three[31m redx"
        );
        let long = format!("{} tail", "\u{4e2d}".repeat(59));
        assert_eq!(
            comment_preview(&long, 60),
            format!("{}…", "\u{4e2d}".repeat(59))
        );
        assert_eq!(comment_preview(&"a".repeat(60), 60), "a".repeat(60));
        assert_eq!(
            comment_preview(&"a".repeat(61), 60),
            format!("{}…", "a".repeat(60))
        );
        assert_eq!(comment_preview("\u{1}\u{2}", 60), "");
        assert_eq!(
            comment_text("a\r\nb\rc\n\td\u{1b}[2J\u{2066}e\u{0}"),
            "a\nb\nc\n\td[2Je"
        );
    }

    #[test]
    fn log_size_limits() {
        let mut big = vec![0x1f, 0x8b];
        big.resize(MAX_LOG + 1, 0);
        let mut v = base();
        v["logs"]["launcher.log"]["data"] = json!(STANDARD.encode(&big));
        assert!(check(&v).unwrap_err().0.contains("larger"));
        big.truncate(MAX_LOG);
        v["logs"]["launcher.log"]["data"] = json!(STANDARD.encode(&big));
        assert!(check(&v).is_ok());
        v["logs"]["second.log"] = json!({"encoding": "gzip+base64", "data": STANDARD.encode(&big)});
        assert!(check(&v).unwrap_err().0.contains("total"));
    }

    #[test]
    fn inflating_is_capped() {
        use std::io::Write;
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(&vec![b'a'; 10_000]).unwrap();
        let gz = enc.finish().unwrap();
        assert_eq!(decompress(&gz, 10_000).unwrap().len(), 10_000);
        assert!(decompress(&gz, 9_999).is_err());
        assert!(decompress(b"\x1f\x8bnot really", 100).is_err());
    }
}
