//! The admin panel's status page under `/admin/status`: every configured game server's status,
//! staff-only ones included, and the maintenance banner shown on `/status` and the home page.
//!
//! Moderators and above can view. Admins and above set or clear the banner; each change goes to
//! the audit log.

use std::sync::Arc;

use askama::Template;
use axum::Router;
use axum::extract::{Form, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect};
use axum::routing::{get, post};
use serde::Deserialize;
use time::OffsetDateTime;

use super::{CsrfOnly, Nav, Staff, require_staff, require_staff_form};
use crate::staff::{self, Role};
use crate::status::{self, StatusView};
use crate::web::{AppState, PageResult, Shared, page, when};

const PATH: &str = "/admin/status";

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(PATH, get(status_page))
        .route("/admin/status/banner", post(set_banner))
        .route("/admin/status/banner/clear", post(clear_banner))
}

#[derive(Template)]
#[template(path = "admin_status.html")]
struct StatusPage {
    nav_csrf: Option<String>,
    csrf: String,
    role: &'static str,
    is_admin: bool,
    view: StatusView,
    configured: bool,
    /// The form's values: the current banner, or what was submitted with an error.
    message: String,
    restart: String,
    error: Option<String>,
}

fn render(
    state: &AppState,
    actor: &Staff,
    error: Option<String>,
    form: Option<&BannerForm>,
) -> PageResult {
    let Nav {
        nav_csrf,
        csrf,
        role,
        is_admin,
    } = actor.nav();
    let view = state.status.view(OffsetDateTime::now_utc(), true);
    let current = state.status.banner();
    let (message, restart) = match form {
        Some(f) => (f.message.clone(), f.restart_at.clone()),
        None => (
            current
                .as_ref()
                .map(|b| b.message.clone())
                .unwrap_or_default(),
            current
                .and_then(|b| b.restart_expected_at)
                .map(|t| {
                    let f =
                        time::macros::format_description!("[year]-[month]-[day]T[hour]:[minute]");
                    t.format(&f).unwrap_or_default()
                })
                .unwrap_or_default(),
        ),
    };
    let status = if error.is_some() {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::OK
    };
    Ok(page(
        status,
        StatusPage {
            nav_csrf,
            csrf,
            role,
            is_admin,
            configured: !view.servers.is_empty(),
            view,
            message,
            restart,
            error,
        },
    ))
}

async fn status_page(State(state): Shared, headers: HeaderMap) -> PageResult {
    let actor = guard!(require_staff(&state, &headers, PATH, Role::Moderator).await);
    render(&state, &actor, None, None)
}

#[derive(Deserialize)]
struct BannerForm {
    csrf: String,
    message: String,
    #[serde(default)]
    restart_at: String,
}

async fn set_banner(
    State(state): Shared,
    headers: HeaderMap,
    Form(form): Form<BannerForm>,
) -> PageResult {
    let actor = guard!(require_staff_form(&state, &headers, Role::Admin, &form.csrf).await);
    let Some(message) = status::banner_message(&form.message) else {
        let error = format!("Write a message of 1 to {} characters.", status::MAX_BANNER);
        return render(&state, &actor, Some(error), Some(&form));
    };
    let Ok(restart) = status::restart_time(&form.restart_at) else {
        let error = "Give the restart time as a date and time (UTC), or leave it empty.".to_owned();
        return render(&state, &actor, Some(error), Some(&form));
    };
    let by = actor.account().email.clone().unwrap_or_default();
    let banner = status::save_banner(&state.pool, &message, restart, &by).await?;
    let detail = match restart {
        Some(t) => format!("{message} (restart expected {})", when(t)),
        None => message,
    };
    staff::log(
        &state.pool,
        actor.account(),
        "set status banner",
        "status page",
        &detail,
    )
    .await?;
    state.status.set_banner(Some(banner));
    Ok(Redirect::to(PATH).into_response())
}

async fn clear_banner(
    State(state): Shared,
    headers: HeaderMap,
    Form(form): Form<CsrfOnly>,
) -> PageResult {
    let actor = guard!(require_staff_form(&state, &headers, Role::Admin, &form.csrf).await);
    if status::clear_banner(&state.pool).await? {
        staff::log(
            &state.pool,
            actor.account(),
            "clear status banner",
            "status page",
            "",
        )
        .await?;
    }
    state.status.set_banner(None);
    Ok(Redirect::to(PATH).into_response())
}
