//! Service accounts under `/admin/service-accounts`: fixed player identities for automation
//! (test clients such as `PlayerLinux`) that never sign in with Google. Staff mint their
//! launcher tokens here instead of linking a launcher.
//!
//! Moderators can see service accounts and their launchers; only admins and owners create,
//! mint, revoke or delete them (and ban them, or unlink their launchers, on the users pages).
//! A minted token is shown once, on the response to the mint form, and stored only hashed.

use std::sync::Arc;

use askama::Template;
use axum::Router;
use axum::extract::{Form, Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use serde::Deserialize;

use super::{CsrfOnly, Nav, Reject, Staff, describe, internal, require_staff, require_staff_form};
use crate::db::{self, ServiceCreateOutcome};
use crate::staff::{self, Role};
use crate::web::{AppState, PageResult, Shared, message, page, when};

const LIST: &str = "/admin/service-accounts";

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(LIST, get(list_page))
        .route("/admin/service-accounts/create", post(create))
        .route("/admin/service-accounts/{id}/mint", post(mint))
        .route(
            "/admin/service-accounts/{id}/devices/{device}/revoke",
            post(revoke),
        )
        .route("/admin/service-accounts/{id}/delete", post(delete))
}

struct DeviceView {
    id: i32,
    label: String,
    created: String,
    last_used: String,
}

struct ServiceView {
    id: i32,
    name: String,
    created: String,
    ban: Option<String>,
    devices: Vec<DeviceView>,
}

#[derive(Template)]
#[template(path = "admin_service_accounts.html")]
struct ListPage {
    nav_csrf: Option<String>,
    csrf: String,
    role: &'static str,
    is_admin: bool,
    accounts: Vec<ServiceView>,
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "admin_service_token.html")]
struct TokenPage {
    nav_csrf: Option<String>,
    role: &'static str,
    is_admin: bool,
    id: i32,
    name: String,
    label: String,
    device_id: i32,
    token: String,
}

async fn render_list(state: &AppState, actor: &Staff, error: Option<String>) -> PageResult {
    let mut accounts = Vec::new();
    for a in db::service_accounts(&state.pool).await? {
        let devices = db::devices(&state.pool, a.id)
            .await?
            .into_iter()
            .map(|d| DeviceView {
                id: d.id,
                label: d.label,
                created: when(d.created_at),
                last_used: d.last_used_at.map(when).unwrap_or_else(|| "never".into()),
            })
            .collect();
        accounts.push(ServiceView {
            id: a.id,
            name: a.player_name.clone().unwrap_or_default(),
            created: when(a.created_at),
            ban: a.banned_at.map(|at| {
                format!(
                    "since {}: {}",
                    when(at),
                    a.ban_reason.as_deref().unwrap_or("")
                )
            }),
            devices,
        });
    }
    let Nav {
        nav_csrf,
        csrf,
        role,
        is_admin,
    } = actor.nav();
    let status = if error.is_some() {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::OK
    };
    Ok(page(
        status,
        ListPage {
            nav_csrf,
            csrf,
            role,
            is_admin,
            accounts,
            error,
        },
    ))
}

async fn list_page(State(state): Shared, headers: HeaderMap) -> PageResult {
    let actor = guard!(require_staff(&state, &headers, LIST, Role::Moderator).await);
    render_list(&state, &actor, None).await
}

fn no_such_account() -> Response {
    message(
        StatusCode::NOT_FOUND,
        "No such service account",
        "That service account does not exist.",
        Some((LIST, "Service accounts")),
    )
}

/// The service account `id`, or the page to show instead.
async fn service_account(state: &AppState, id: i32) -> Result<db::Account, Reject> {
    match db::account(&state.pool, id).await {
        Ok(Some(a)) if a.is_service() => Ok(a),
        Ok(_) => Err(Box::new(no_such_account())),
        Err(e) => Err(Box::new(internal(e))),
    }
}

#[derive(Deserialize)]
struct CreateForm {
    csrf: String,
    name: String,
}

async fn create(
    State(state): Shared,
    headers: HeaderMap,
    Form(form): Form<CreateForm>,
) -> PageResult {
    let actor = guard!(require_staff_form(&state, &headers, Role::Admin, &form.csrf).await);
    let name = form.name.trim();
    let problem = match db::create_service_account(&state.pool, name).await? {
        ServiceCreateOutcome::Created {
            account_id,
            name,
            released,
        } => {
            let detail = if released {
                "took over the reserved name"
            } else {
                ""
            };
            staff::log(
                &state.pool,
                actor.account(),
                "create service account",
                &format!("account {account_id} ({name})"),
                detail,
            )
            .await?;
            return Ok(Redirect::to(LIST).into_response());
        }
        ServiceCreateOutcome::Invalid => {
            "Use 3 to 15 letters, digits, - or _, starting with a letter."
        }
        ServiceCreateOutcome::Taken => {
            "An account already holds that name. Service accounts take only free or reserved names."
        }
    };
    render_list(&state, &actor, Some(problem.to_owned())).await
}

#[derive(Deserialize)]
struct MintForm {
    csrf: String,
    label: String,
}

async fn mint(
    State(state): Shared,
    headers: HeaderMap,
    Path(id): Path<i32>,
    Form(form): Form<MintForm>,
) -> PageResult {
    let actor = guard!(require_staff_form(&state, &headers, Role::Admin, &form.csrf).await);
    let account = guard!(service_account(&state, id).await);
    let Some(label) = db::service_token_label(&form.label) else {
        return render_list(
            &state,
            &actor,
            Some("Give the token a label of 1 to 64 printable characters, such as the machine it runs on.".into()),
        )
        .await;
    };
    // `None`: deleted since the lookup above.
    let Some(minted) = db::mint_service_token(&state.pool, id, &label).await? else {
        return Ok(no_such_account());
    };
    staff::log(
        &state.pool,
        actor.account(),
        "mint service token",
        &describe(&account),
        &format!("device {}: {label}", minted.device_id),
    )
    .await?;
    let Nav {
        nav_csrf,
        role,
        is_admin,
        ..
    } = actor.nav();
    let mut response = page(
        StatusCode::OK,
        TokenPage {
            nav_csrf,
            role,
            is_admin,
            id,
            name: account.player_name.unwrap_or_default(),
            label,
            device_id: minted.device_id,
            token: minted.token,
        },
    );
    // The only copy of the token: no cache may keep it.
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    Ok(response)
}

async fn revoke(
    State(state): Shared,
    headers: HeaderMap,
    Path((id, device)): Path<(i32, i32)>,
    Form(form): Form<CsrfOnly>,
) -> PageResult {
    let actor = guard!(require_staff_form(&state, &headers, Role::Admin, &form.csrf).await);
    let account = guard!(service_account(&state, id).await);
    if db::revoke_device(&state.pool, id, device).await? {
        staff::log(
            &state.pool,
            actor.account(),
            "revoke service token",
            &describe(&account),
            &format!("device {device}"),
        )
        .await?;
    }
    Ok(Redirect::to(LIST).into_response())
}

#[derive(Deserialize)]
struct DeleteForm {
    csrf: String,
    confirm: String,
}

async fn delete(
    State(state): Shared,
    headers: HeaderMap,
    Path(id): Path<i32>,
    Form(form): Form<DeleteForm>,
) -> PageResult {
    let actor = guard!(require_staff_form(&state, &headers, Role::Admin, &form.csrf).await);
    let account = guard!(service_account(&state, id).await);
    let name = account.player_name.as_deref().unwrap_or_default();
    if !form.confirm.trim().eq_ignore_ascii_case(name) {
        return Ok(message(
            StatusCode::BAD_REQUEST,
            "Service account kept",
            "Type the account's name to confirm. Nothing was deleted.",
            Some((LIST, "Back")),
        ));
    }
    db::delete_account(&state.pool, id).await?;
    staff::log(
        &state.pool,
        actor.account(),
        "delete service account",
        &describe(&account),
        "",
    )
    .await?;
    Ok(Redirect::to(LIST).into_response())
}
