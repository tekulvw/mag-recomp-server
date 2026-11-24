//! Launcher reports under `/admin/reports`: daily counts, a filtered list of recent reports
//! and each report's metadata and metrics. Staff from moderator up see them; only admins
//! download logs (decompressed, as text), and each download goes to the audit log.

use std::sync::Arc;

use askama::Template;
use axum::Router;
use axum::extract::{Form, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::Deserialize;
use time::{Duration, OffsetDateTime};

use super::{CsrfOnly, Nav, require_staff, require_staff_form};
use crate::reports::{self, Filter, PLATFORMS, TRIGGERS};
use crate::staff::{self, Role};
use crate::web::{AppState, PageResult, Shared, message, page, when};

const LIST: &str = "/admin/reports";
const STATS_DAYS: i64 = 14;
const LIST_LIMIT: i64 = 100;

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(LIST, get(list_page))
        .route("/admin/reports/{id}", get(report_page))
        .route("/admin/reports/{id}/logs/{name}", post(download_log))
}

struct DayView {
    day: String,
    by_trigger: Vec<i64>,
    by_platform: Vec<i64>,
    total: i64,
}

struct ReportView {
    id: String,
    received: String,
    account_id: i32,
    player: String,
    trigger: String,
    platform: String,
    version: String,
    logs: usize,
    comment: String,
}

#[derive(Template)]
#[template(path = "admin_reports.html")]
struct ListPage {
    nav_csrf: Option<String>,
    role: &'static str,
    is_admin: bool,
    enabled: bool,
    retention_days: i64,
    triggers: &'static [&'static str],
    platforms: &'static [&'static str],
    days: Vec<DayView>,
    account: String,
    trigger: String,
    platform: String,
    has_comment: bool,
    rows: Vec<ReportView>,
}

#[derive(Deserialize)]
struct ListQuery {
    account: Option<String>,
    trigger: Option<String>,
    platform: Option<String>,
    /// `yes`: only reports with a player comment.
    comment: Option<String>,
}

/// A filter value from the query: trimmed, empty means none.
fn chosen(value: Option<String>, allowed: Option<&[&str]>) -> Option<String> {
    let v: String = value?.trim().chars().take(32).collect();
    match allowed {
        _ if v.is_empty() => None,
        Some(list) if !list.contains(&v.as_str()) => None,
        _ => Some(v),
    }
}

fn days_view(counts: Vec<reports::DayCount>) -> Vec<DayView> {
    let today = OffsetDateTime::now_utc();
    (0..STATS_DAYS)
        .map(|back| {
            let day = crate::report_store::day(today - Duration::days(back));
            let of_day: Vec<&reports::DayCount> = counts.iter().filter(|c| c.day == day).collect();
            let sum = |pick: &dyn Fn(&reports::DayCount) -> bool| -> i64 {
                of_day.iter().filter(|c| pick(c)).map(|c| c.count).sum()
            };
            DayView {
                by_trigger: TRIGGERS.iter().map(|t| sum(&|c| c.trigger == *t)).collect(),
                by_platform: PLATFORMS
                    .iter()
                    .map(|p| sum(&|c| c.platform == *p))
                    .collect(),
                total: sum(&|_| true),
                day,
            }
        })
        .collect()
}

async fn list_page(
    State(state): Shared,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> PageResult {
    let actor = guard!(require_staff(&state, &headers, LIST, Role::Moderator).await);
    let filter = Filter {
        account: chosen(query.account, None),
        trigger: chosen(query.trigger, Some(&TRIGGERS)),
        platform: chosen(query.platform, Some(&PLATFORMS)),
        since: None,
        has_comment: query.comment.as_deref() == Some("yes"),
    };
    let days = days_view(reports::stats(&state.pool, STATS_DAYS).await?);
    let rows = reports::list(&state.pool, &filter, LIST_LIMIT)
        .await?
        .into_iter()
        .map(|r| ReportView {
            received: when(r.received_at),
            player: r.player_name.unwrap_or_default(),
            logs: r.log_names.len(),
            comment: r
                .user_comment
                .as_deref()
                .map(|c| reports::comment_preview(c, reports::COMMENT_PREVIEW))
                .unwrap_or_default(),
            id: r.id,
            account_id: r.account_id,
            trigger: r.trigger,
            platform: r.platform,
            version: r.client_version,
        })
        .collect();
    let Nav {
        nav_csrf,
        role,
        is_admin,
        ..
    } = actor.nav();
    Ok(page(
        StatusCode::OK,
        ListPage {
            nav_csrf,
            role,
            is_admin,
            enabled: state.reports.is_some(),
            retention_days: state.reports.as_ref().map_or(0, |r| r.retention_days),
            triggers: &TRIGGERS,
            platforms: &PLATFORMS,
            days,
            account: filter.account.unwrap_or_default(),
            trigger: filter.trigger.unwrap_or_default(),
            platform: filter.platform.unwrap_or_default(),
            has_comment: filter.has_comment,
            rows,
        },
    ))
}

#[derive(Template)]
#[template(path = "admin_report.html")]
struct ReportPage {
    nav_csrf: Option<String>,
    csrf: String,
    role: &'static str,
    is_admin: bool,
    id: String,
    received: String,
    account_id: i32,
    player: String,
    account_kind: String,
    trigger: String,
    platform: String,
    version: String,
    log_names: Vec<String>,
    log_bytes: i64,
    comment: Option<String>,
    client: String,
    session: Option<String>,
    metrics: Option<String>,
}

fn pretty(json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(json)
        .and_then(|v| serde_json::to_string_pretty(&v))
        .unwrap_or_else(|_| json.to_owned())
}

fn no_such_report() -> Response {
    message(
        StatusCode::NOT_FOUND,
        "No such report",
        "That report does not exist, or has expired.",
        Some((LIST, "Reports")),
    )
}

async fn report_page(
    State(state): Shared,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> PageResult {
    let path = format!("{LIST}/{}", id.chars().take(36).collect::<String>());
    let actor = guard!(require_staff(&state, &headers, &path, Role::Moderator).await);
    let Some(d) = reports::detail(&state.pool, &id).await? else {
        return Ok(no_such_report());
    };
    let Nav {
        nav_csrf,
        csrf,
        role,
        is_admin,
    } = actor.nav();
    let r = d.row;
    Ok(page(
        StatusCode::OK,
        ReportPage {
            nav_csrf,
            csrf,
            role,
            is_admin,
            id: r.id,
            received: when(r.received_at),
            account_id: r.account_id,
            player: r.player_name.unwrap_or_default(),
            account_kind: r.account_kind,
            trigger: r.trigger,
            platform: r.platform,
            version: r.client_version,
            log_names: r.log_names,
            log_bytes: r.log_bytes,
            comment: r.user_comment.as_deref().map(reports::comment_text),
            client: pretty(&d.client),
            session: d.session.as_deref().map(pretty),
            metrics: d.metrics.as_deref().map(pretty),
        },
    ))
}

async fn download_log(
    State(state): Shared,
    headers: HeaderMap,
    Path((id, name)): Path<(String, String)>,
    Form(form): Form<CsrfOnly>,
) -> PageResult {
    let actor = guard!(require_staff_form(&state, &headers, Role::Admin, &form.csrf).await);
    let Some(d) = reports::detail(&state.pool, &id).await? else {
        return Ok(no_such_report());
    };
    let Some(service) = state.reports.as_ref() else {
        return Ok(message(
            StatusCode::SERVICE_UNAVAILABLE,
            "Report store off",
            "Reports are turned off on this server (PORTAL_REPORTS), so their logs cannot be read here.",
            Some((LIST, "Reports")),
        ));
    };
    let back = format!("{LIST}/{}", d.row.id);
    let Some(gz) = reports::log(service.store.as_ref(), &d.row, &name).await? else {
        return Ok(message(
            StatusCode::NOT_FOUND,
            "No such log",
            "That report has no log by that name, or it is gone from the store.",
            Some((&back, "Back")),
        ));
    };
    let text = match reports::decompress(&gz, reports::MAX_DECOMPRESSED) {
        Ok(text) => text,
        Err(e) => {
            return Ok(message(
                StatusCode::UNPROCESSABLE_ENTITY,
                "Log unreadable",
                &format!("The stored log could not be decompressed: {e}."),
                Some((&back, "Back")),
            ));
        }
    };
    staff::log(
        &state.pool,
        actor.account(),
        "download report log",
        &format!("report {}", d.row.id),
        &name,
    )
    .await?;
    // `name` matched a stored log name, so it holds only [a-z0-9._-].
    let disposition = format!("attachment; filename=\"report-{}-{name}.txt\"", d.row.id);
    let mut response = (StatusCode::OK, text).into_response();
    let h = response.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    h.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::try_from(disposition)?,
    );
    Ok(response)
}
