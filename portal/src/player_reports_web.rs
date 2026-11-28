//! The player side of player reports (docs/portal-player-reports.md): "Report a player" at
//! `/reports/new`, the reporter's own list at `/reports`, and name suggestions for the form at
//! `/reports/players`. Moderators work through reports in the admin panel
//! (`admin_web::player_reports`).
//!
//! Reporters see character names only: never account numbers, emails or sign-in names, and
//! never what a moderator wrote or decided.

use std::sync::Arc;

use askama::Template;
use axum::extract::{Form, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use time::{Duration, OffsetDateTime};

use crate::db::Session;
use crate::player_reports::{
    self, MAX_DESCRIPTION, NewReport, Reason, Resolved, Status, SubmitOutcome,
};
use crate::web::{
    AppState, PageResult, Shared, Stamp, api_error, csrf_failed, csrf_ok, current_session, message,
    page, require_session, stamp,
};

const SCRIPT: &str = include_str!("../static/report.js");
const MINE_LIMIT: i64 = 100;
/// How far back an incident time may be.
const MAX_AGE_DAYS: i64 = 365;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/reports", get(mine_page))
        .route("/reports/new", get(form_page).post(submit))
        .route("/reports/players", get(players))
        .route("/report.js", get(script))
}

/// A short digest of the form script, for `/report.js?v=<this>` (as for the stylesheet).
pub fn script_version() -> &'static str {
    static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VERSION.get_or_init(|| crate::tokens::b64url(&crate::tokens::sha256(SCRIPT.as_bytes())[..9]))
}

#[derive(Deserialize)]
struct ScriptQuery {
    v: Option<String>,
}

/// The optional script that suggests names as the reporter types. Without it, the form's
/// search button lists matching names instead.
async fn script(Query(q): Query<ScriptQuery>) -> Response {
    let cache = if q.v.as_deref() == Some(script_version()) {
        "public, max-age=31536000, immutable"
    } else {
        "public, max-age=300"
    };
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, cache),
        ],
        SCRIPT,
    )
        .into_response()
}

macro_rules! guard {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(response) => return Ok(*response),
        }
    };
}

/// A signed-in player with a name and no ban, or the page to show instead.
async fn reporter(
    state: &AppState,
    headers: &HeaderMap,
    next: &str,
) -> Result<Session, Box<Response>> {
    let session = require_session(state, headers, next).await?;
    if session.account.player_name.is_none() {
        return Err(Box::new(
            Redirect::to(&format!(
                "/claim?next={}",
                crate::oidc::form_urlencode(next)
            ))
            .into_response(),
        ));
    }
    if session.account.is_banned() {
        return Err(Box::new(message(
            StatusCode::FORBIDDEN,
            "Account banned",
            "This account is banned, so it cannot report players.",
            Some(("/account", "Your account")),
        )));
    }
    Ok(session)
}

fn own_key(session: &Session) -> Option<String> {
    session
        .account
        .game_account_name
        .as_deref()
        .map(crate::names::name_key)
}

// ---- My reports ----

struct MyRow {
    target: String,
    reason: String,
    filed: Stamp,
    status: &'static str,
}

#[derive(Template)]
#[template(path = "reports_mine.html")]
struct MinePage {
    nav_csrf: Option<String>,
    rows: Vec<MyRow>,
    filed: bool,
}

#[derive(Deserialize)]
struct MineQuery {
    filed: Option<String>,
}

async fn mine_page(
    State(state): Shared,
    headers: HeaderMap,
    Query(q): Query<MineQuery>,
) -> PageResult {
    let session = guard!(reporter(&state, &headers, "/reports").await);
    let rows = player_reports::mine(&state.pool, session.account.id, MINE_LIMIT)
        .await?
        .into_iter()
        .map(|r| MyRow {
            target: r.target_name,
            reason: Reason::label_of(&r.reason).to_owned(),
            filed: stamp(r.created_at),
            status: Status::reporter_view(&r.status),
        })
        .collect();
    Ok(page(
        StatusCode::OK,
        MinePage {
            nav_csrf: Some(session.csrf_token),
            rows,
            filed: q.filed.is_some(),
        },
    ))
}

// ---- Report a player ----

#[derive(Template)]
#[template(path = "report_new.html")]
struct FormPage {
    nav_csrf: Option<String>,
    csrf: String,
    reasons: Vec<(&'static str, &'static str)>,
    name: String,
    reason: String,
    description: String,
    when: String,
    error: Option<String>,
    /// The search box's text and the names it found, for browsers without the script.
    query: String,
    results: Vec<String>,
    searched: bool,
    per_day: i64,
    max_description: usize,
    script_version: &'static str,
}

impl FormPage {
    fn new(state: &AppState, session: &Session) -> Self {
        Self {
            nav_csrf: Some(session.csrf_token.clone()),
            csrf: session.csrf_token.clone(),
            reasons: Reason::ALL
                .iter()
                .map(|r| (r.as_str(), r.label()))
                .collect(),
            name: String::new(),
            reason: String::new(),
            description: String::new(),
            when: String::new(),
            error: None,
            query: String::new(),
            results: Vec::new(),
            searched: false,
            per_day: state.settings.player_reports_per_day,
            max_description: MAX_DESCRIPTION,
            script_version: script_version(),
        }
    }
}

#[derive(Deserialize)]
struct FormQuery {
    /// A name to start the form with.
    name: Option<String>,
    /// Search text, from the search box.
    q: Option<String>,
}

fn short(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

async fn form_page(
    State(state): Shared,
    headers: HeaderMap,
    Query(q): Query<FormQuery>,
) -> PageResult {
    let session = guard!(reporter(&state, &headers, "/reports/new").await);
    let mut view = FormPage::new(&state, &session);
    view.name = short(q.name.as_deref().unwrap_or("").trim(), 32);
    if let Some(query) = q.q {
        if !state.limits.player_search.allow_key(session.account.id) {
            return Ok(too_fast());
        }
        view.query = short(query.trim(), 32);
        view.searched = true;
        view.results = player_reports::suggest(
            &state.pool,
            state.settings.game_admin.as_ref(),
            &view.query,
            own_key(&session).as_deref(),
        )
        .await?;
    }
    Ok(page(StatusCode::OK, view))
}

fn too_fast() -> Response {
    message(
        StatusCode::TOO_MANY_REQUESTS,
        "Slow down",
        "Too many name searches in a minute. Wait a moment, then try again.",
        Some(("/reports/new", "Report a player")),
    )
}

#[derive(Deserialize)]
struct ReportForm {
    csrf: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    reason: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    when: String,
    /// The browser's `Date.getTimezoneOffset()` (minutes behind UTC), filled in by the form
    /// script; without it the time is taken as UTC.
    #[serde(default)]
    tz: String,
}

/// The incident time from the form: `YYYY-MM-DDTHH:MM` (seconds optional) in the reporter's
/// time zone, as UTC. `Ok(None)` when left empty.
fn parse_when(raw: &str, tz: &str, now: OffsetDateTime) -> Result<Option<OffsetDateTime>, ()> {
    let Some(at) = crate::web::form_instant(raw, tz)? else {
        return Ok(None);
    };
    // A day of slack for a browser clock or time zone that is off.
    if at > now + Duration::days(1) || at < now - Duration::days(MAX_AGE_DAYS) {
        return Err(());
    }
    Ok(Some(at))
}

async fn submit(
    State(state): Shared,
    headers: HeaderMap,
    Form(form): Form<ReportForm>,
) -> PageResult {
    let session = guard!(reporter(&state, &headers, "/reports/new").await);
    if !csrf_ok(&session, &form.csrf) {
        return Ok(csrf_failed());
    }
    let description = player_reports::clean_text(&form.description);
    let mut view = FormPage::new(&state, &session);
    view.name = short(form.name.trim(), 32);
    view.reason = short(form.reason.trim(), 32);
    view.description = short(&description, MAX_DESCRIPTION + 100);
    view.when = short(form.when.trim(), 32);
    let refuse = |mut view: FormPage, status: StatusCode, text: &str| {
        view.error = Some(text.to_owned());
        Ok(page(status, view))
    };
    let Some(key) = player_reports::target_key(&view.name) else {
        return refuse(
            view,
            StatusCode::BAD_REQUEST,
            "Enter the name of the player you are reporting.",
        );
    };
    if own_key(&session).as_deref() == Some(key.as_str()) {
        return refuse(view, StatusCode::BAD_REQUEST, "You cannot report yourself.");
    }
    let Some(reason) = Reason::parse(&view.reason) else {
        return refuse(view, StatusCode::BAD_REQUEST, "Choose a reason.");
    };
    if description.is_empty() {
        return refuse(
            view,
            StatusCode::BAD_REQUEST,
            "Describe what happened, so a moderator can look into it.",
        );
    }
    if description.chars().count() > MAX_DESCRIPTION {
        return refuse(
            view,
            StatusCode::BAD_REQUEST,
            &format!("Keep the description to {MAX_DESCRIPTION} characters."),
        );
    }
    let Ok(occurred_at) = parse_when(&form.when, &form.tz, OffsetDateTime::now_utc()) else {
        return refuse(
            view,
            StatusCode::BAD_REQUEST,
            "Give a time within the last year, or leave it empty.",
        );
    };
    let target =
        match player_reports::resolve(&state.pool, state.settings.game_admin.as_ref(), &key).await?
        {
            Resolved::Found(t) => t,
            Resolved::Unknown => {
                let text = format!("Nobody is called {}.", view.name);
                return refuse(view, StatusCode::BAD_REQUEST, &text);
            }
            Resolved::Unavailable => {
                return refuse(
                    view,
                    StatusCode::SERVICE_UNAVAILABLE,
                    "The game server cannot be reached to check that name. Try again in a moment.",
                );
            }
        };
    if target.account == Some(session.account.id) {
        return refuse(view, StatusCode::BAD_REQUEST, "You cannot report yourself.");
    }
    let outcome = player_reports::submit(
        &state.pool,
        &NewReport {
            reporter: session.account.id,
            target: &target,
            reason,
            description: &description,
            occurred_at,
        },
        state.settings.player_reports_per_day,
    )
    .await?;
    match outcome {
        SubmitOutcome::Filed(id) => {
            tracing::info!(report = id, "player report filed");
            Ok(Redirect::to("/reports?filed=1").into_response())
        }
        SubmitOutcome::Duplicate => {
            let text = format!(
                "You already have an open report about {}. A moderator will look at it; you can report them again once it has been reviewed.",
                target.name
            );
            refuse(view, StatusCode::CONFLICT, &text)
        }
        SubmitOutcome::RateLimited => {
            let text = format!(
                "You can file {} reports in 24 hours. Try again later.",
                state.settings.player_reports_per_day
            );
            refuse(view, StatusCode::TOO_MANY_REQUESTS, &text)
        }
    }
}

// ---- Name suggestions ----

#[derive(Deserialize)]
struct PlayersQuery {
    q: Option<String>,
}

/// `{"players": [names]}` for the form script. Signed-in players only, within a per-account
/// limit; names only.
async fn players(
    State(state): Shared,
    headers: HeaderMap,
    Query(q): Query<PlayersQuery>,
) -> PageResult {
    let session = match current_session(&state, &headers).await? {
        Some(s) if s.account.player_name.is_some() && !s.account.is_banned() => s,
        _ => return Ok(api_error(StatusCode::UNAUTHORIZED, "not_signed_in")),
    };
    if !state.limits.player_search.allow_key(session.account.id) {
        return Ok((
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, "60")],
            Json(json!({ "error": "rate_limited" })),
        )
            .into_response());
    }
    let names = player_reports::suggest(
        &state.pool,
        state.settings.game_admin.as_ref(),
        q.q.as_deref().unwrap_or(""),
        own_key(&session).as_deref(),
    )
    .await?;
    Ok(Json(json!({ "players": names })).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incident_times() {
        let now = time::macros::datetime!(2026-11-27 12:00 UTC);
        assert_eq!(parse_when("", "", now), Ok(None));
        assert_eq!(
            parse_when("2026-11-27T10:30", "", now),
            Ok(Some(time::macros::datetime!(2026-11-27 10:30 UTC)))
        );
        // Five hours behind UTC, as getTimezoneOffset() says it.
        assert_eq!(
            parse_when("2026-11-27T05:30:00", "300", now),
            Ok(Some(time::macros::datetime!(2026-11-27 10:30 UTC)))
        );
        assert_eq!(
            parse_when("2026-11-27T13:30", "-60", now),
            Ok(Some(time::macros::datetime!(2026-11-27 12:30 UTC)))
        );
        for (bad, tz) in [
            ("yesterday", ""),
            ("2026-11-27 10:30", ""),
            ("2026-11-29T12:00", ""),
            ("2025-01-01T00:00", ""),
            ("2026-11-27T10:30", "x"),
            ("2026-11-27T10:30", "2000"),
        ] {
            assert_eq!(parse_when(bad, tz, now), Err(()), "{bad} {tz}");
        }
    }
}
