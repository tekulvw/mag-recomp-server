//! Player reports under `/admin/player-reports` (docs/portal-player-reports.md): the queue,
//! filtered by status and reason, and each report with the other reports about the same
//! character. Moderators and above mark a report actioned or dismissed (with a note) or reopen
//! it; every change goes to the audit log. Penalties stay with the existing tools: the
//! character page (voice mute and ban, levels) and the user page (account ban).

use std::sync::Arc;

use askama::Template;
use axum::Router;
use axum::extract::{Form, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use serde::Deserialize;

use super::{Nav, Staff, require_staff, require_staff_form};
use crate::player_reports::{self, MAX_NOTE, Reason, Report, ReviewOutcome, Status};
use crate::staff::{self, Role};
use crate::web::{AppState, PageResult, Shared, message, page, when};

const LIST: &str = "/admin/player-reports";
const LIST_LIMIT: i64 = 200;
const OTHERS_LIMIT: i64 = 50;
const PREVIEW: usize = 120;

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(LIST, get(list_page))
        .route("/admin/player-reports/{id}", get(report_page))
        .route("/admin/player-reports/{id}/review", post(review))
}

fn preview(text: &str) -> String {
    let mut out: String = text
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .take(PREVIEW)
        .collect();
    if text.chars().count() > PREVIEW {
        out.push('…');
    }
    out
}

struct Row {
    id: i64,
    filed: String,
    reporter_id: Option<i32>,
    reporter: String,
    target: String,
    reason: String,
    status: String,
    preview: String,
    others: i64,
    others_open: i64,
}

impl From<Report> for Row {
    fn from(r: Report) -> Self {
        Self {
            filed: when(r.created_at),
            reporter: reporter_label(r.reporter_account, r.reporter_name.as_deref()),
            reporter_id: r.reporter_account,
            target: r.target_name,
            reason: Reason::label_of(&r.reason).to_owned(),
            preview: preview(&r.description),
            id: r.id,
            status: r.status,
            others: r.other_reports,
            others_open: r.other_open,
        }
    }
}

fn reporter_label(id: Option<i32>, name: Option<&str>) -> String {
    match (id, name) {
        (Some(_), Some(name)) => name.to_owned(),
        (Some(id), None) => format!("account {id}"),
        (None, _) => "deleted account".to_owned(),
    }
}

#[derive(Template)]
#[template(path = "admin_player_reports.html")]
struct ListPage {
    nav_csrf: Option<String>,
    role: &'static str,
    is_admin: bool,
    statuses: [&'static str; 4],
    status: String,
    reasons: Vec<(&'static str, &'static str)>,
    reason: String,
    open_total: i64,
    rows: Vec<Row>,
}

#[derive(Deserialize)]
struct ListQuery {
    /// `open` (the default), `actioned`, `dismissed` or `all`.
    status: Option<String>,
    reason: Option<String>,
}

async fn list_page(
    State(state): Shared,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> PageResult {
    let actor = guard!(require_staff(&state, &headers, LIST, Role::Moderator).await);
    let status_text = query.status.as_deref().unwrap_or("open").trim();
    let (status, status_text) = match Status::parse(status_text) {
        Some(s) => (Some(s), s.as_str()),
        None if status_text == "all" => (None, "all"),
        None => (Some(Status::Open), "open"),
    };
    let reason = query
        .reason
        .as_deref()
        .map(str::trim)
        .and_then(Reason::parse);
    let rows = player_reports::queue(&state.pool, status, reason, LIST_LIMIT)
        .await?
        .into_iter()
        .map(Row::from)
        .collect();
    let Nav {
        nav_csrf,
        csrf: _,
        role,
        is_admin,
    } = actor.nav();
    Ok(page(
        StatusCode::OK,
        ListPage {
            nav_csrf,
            role,
            is_admin,
            statuses: ["open", "actioned", "dismissed", "all"],
            status: status_text.to_owned(),
            reasons: Reason::ALL
                .iter()
                .map(|r| (r.as_str(), r.label()))
                .collect(),
            reason: reason.map(|r| r.as_str().to_owned()).unwrap_or_default(),
            open_total: player_reports::open_count(&state.pool).await?,
            rows,
        },
    ))
}

#[derive(Template)]
#[template(path = "admin_player_report.html")]
struct ReportPage {
    nav_csrf: Option<String>,
    csrf: String,
    role: &'static str,
    is_admin: bool,
    id: i64,
    filed: String,
    reporter_id: Option<i32>,
    reporter: String,
    target_name: String,
    /// The portal account holding the character's name now.
    target_account: Option<i32>,
    game_href: Option<String>,
    reason: String,
    description: String,
    occurred: Option<String>,
    status: String,
    open: bool,
    reviewed: Option<String>,
    note: Option<String>,
    others: Vec<Row>,
    /// Why this staff member cannot change the report, if they cannot.
    blocked: Option<&'static str>,
    max_note: usize,
}

/// Why `actor` may not review reports about the character `target_key`, if they may not: its
/// holder is staff ranked at or above them (or is them).
async fn review_block(
    state: &AppState,
    actor: &Staff,
    target_key: &str,
) -> anyhow::Result<(Option<i32>, Option<&'static str>)> {
    let Some(account) = crate::db::account_by_name(&state.pool, target_key).await? else {
        return Ok((None, None));
    };
    let role = staff::role_for_email(
        &state.pool,
        &state.settings.owner_emails,
        account.email.as_deref(),
    )
    .await?;
    let blocked = (!staff::outranks(actor.role, role)).then_some(
        "This player is staff ranked at or above you, so someone ranked above them reviews it.",
    );
    Ok((Some(account.id), blocked))
}

async fn report_page(State(state): Shared, headers: HeaderMap, Path(id): Path<i64>) -> PageResult {
    let path = format!("{LIST}/{id}");
    let actor = guard!(require_staff(&state, &headers, &path, Role::Moderator).await);
    let Some(r) = player_reports::report(&state.pool, id).await? else {
        return Ok(no_report());
    };
    let (target_account, blocked) = review_block(&state, &actor, &r.target_key).await?;
    let others = player_reports::about(&state.pool, &r.target_key, id, OTHERS_LIMIT)
        .await?
        .into_iter()
        .map(Row::from)
        .collect();
    let reviewed = r.reviewed_at.map(|at| {
        format!(
            "{} by {}",
            when(at),
            r.reviewed_by_email
                .as_deref()
                .filter(|e| !e.is_empty())
                .unwrap_or("staff")
        )
    });
    let Nav {
        nav_csrf,
        csrf,
        role,
        is_admin,
    } = actor.nav();
    Ok(page(
        StatusCode::OK,
        ReportPage {
            nav_csrf,
            csrf,
            role,
            is_admin,
            id,
            filed: when(r.created_at),
            reporter: reporter_label(r.reporter_account, r.reporter_name.as_deref()),
            reporter_id: r.reporter_account,
            game_href: super::game::character_href(
                state.settings.game_admin.as_ref(),
                Some(&r.target_key),
            ),
            target_name: r.target_name,
            target_account,
            reason: Reason::label_of(&r.reason).to_owned(),
            description: r.description,
            occurred: r.occurred_at.map(when),
            open: r.status == Status::Open.as_str(),
            status: r.status,
            reviewed,
            note: r.review_note,
            others,
            blocked,
            max_note: MAX_NOTE,
        },
    ))
}

fn no_report() -> Response {
    message(
        StatusCode::NOT_FOUND,
        "No such report",
        "That player report does not exist.",
        Some((LIST, "Player reports")),
    )
}

#[derive(Deserialize)]
struct ReviewForm {
    csrf: String,
    /// `actioned`, `dismissed` or `open` (reopen).
    action: String,
    #[serde(default)]
    note: String,
}

async fn review(
    State(state): Shared,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Form(form): Form<ReviewForm>,
) -> PageResult {
    let actor = guard!(require_staff_form(&state, &headers, Role::Moderator, &form.csrf).await);
    let back = format!("{LIST}/{id}");
    let bad = |title: &str, text: &str| {
        message(
            StatusCode::BAD_REQUEST,
            title,
            &format!("{text} Nothing was changed."),
            Some((&back, "Back to the report")),
        )
    };
    let Some(to) = Status::parse(form.action.trim()) else {
        return Ok(bad(
            "Choose an action",
            "Mark the report actioned or dismissed, or reopen it.",
        ));
    };
    let note = player_reports::clean_text(&form.note);
    if to != Status::Open && note.is_empty() {
        return Ok(bad(
            "Note needed",
            "Say what you did or why you dismissed it; only staff see the note.",
        ));
    }
    if note.chars().count() > MAX_NOTE {
        return Ok(bad(
            "Note too long",
            &format!("Keep the note to {MAX_NOTE} characters."),
        ));
    }
    let Some(r) = player_reports::report(&state.pool, id).await? else {
        return Ok(no_report());
    };
    if let (_, Some(why)) = review_block(&state, &actor, &r.target_key).await? {
        return Ok(message(
            StatusCode::FORBIDDEN,
            "Not allowed",
            why,
            Some((&back, "Back to the report")),
        ));
    }
    Ok(
        match player_reports::review(&state.pool, id, to, &note, actor.account()).await? {
            ReviewOutcome::Changed | ReviewOutcome::Unchanged => {
                Redirect::to(&back).into_response()
            }
            ReviewOutcome::NoSuchReport => no_report(),
            ReviewOutcome::ReporterHasAnotherOpen => message(
                StatusCode::CONFLICT,
                "Report not reopened",
                "The reporter has a newer open report about this player. Review that one instead.",
                Some((&back, "Back to the report")),
            ),
        },
    )
}
