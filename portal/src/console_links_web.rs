//! Signed-in account console controls. Codes are accepted only in POST bodies, never URLs.

use std::sync::Arc;

use askama::Template;
use axum::Router;
use axum::extract::{DefaultBodyLimit, Form, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect};
use axum::routing::{get, post};
use serde::Deserialize;

use crate::console_links::{self, Identity, RedeemOutcome};
use crate::db::Session;
use crate::web::{AppState, PageResult, Shared, csrf_failed, csrf_ok, page, require_session};

const PATH: &str = "/account/console";

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(PATH, get(console_page).post(redeem))
        .route("/account/console/unlink", post(unlink))
        .layer(DefaultBodyLimit::max(4096))
}

#[derive(Template)]
#[template(path = "console.html")]
struct ConsolePage {
    nav_csrf: Option<String>,
    csrf: String,
    identities: Vec<Identity>,
    can_link: bool,
    notice: Option<&'static str>,
}

async fn render(
    state: &AppState,
    session: Session,
    status: StatusCode,
    notice: Option<&'static str>,
) -> PageResult {
    let identities = console_links::list(&state.pool, session.account.id).await?;
    let can_link = !session.account.is_banned() && session.account.player_name.is_some();
    Ok(page(
        status,
        ConsolePage {
            nav_csrf: Some(session.csrf_token.clone()),
            csrf: session.csrf_token,
            identities,
            can_link,
            notice,
        },
    ))
}

async fn console_page(State(state): Shared, headers: HeaderMap) -> PageResult {
    let session = match require_session(&state, &headers, PATH).await {
        Ok(s) => s,
        Err(r) => return Ok(*r),
    };
    render(&state, session, StatusCode::OK, None).await
}

#[derive(Deserialize)]
struct RedeemForm {
    csrf: String,
    code: String,
}

async fn redeem(
    State(state): Shared,
    headers: HeaderMap,
    Form(form): Form<RedeemForm>,
) -> PageResult {
    let session = match require_session(&state, &headers, PATH).await {
        Ok(s) => s,
        Err(r) => return Ok(*r),
    };
    if !csrf_ok(&session, &form.csrf) {
        return Ok(csrf_failed());
    }
    match console_links::redeem(&state.pool, session.account.id, &form.code).await? {
        RedeemOutcome::Linked => Ok(Redirect::to(PATH).into_response()),
        RedeemOutcome::InvalidProof => render(&state, session, StatusCode::BAD_REQUEST,
            Some("That console code is invalid, expired or already used. Request a new code from your console.")).await,
        RedeemOutcome::Conflict => render(&state, session, StatusCode::CONFLICT,
            Some("That Sony identity or your portal account already has a console link. Unlink the existing identity before trying again.")).await,
        RedeemOutcome::IneligibleAccount => render(&state, session, StatusCode::FORBIDDEN,
            Some("Only an active person account with a claimed player name can link a console.")).await,
    }
}

#[derive(Deserialize)]
struct UnlinkForm {
    csrf: String,
}

async fn unlink(
    State(state): Shared,
    headers: HeaderMap,
    Form(form): Form<UnlinkForm>,
) -> PageResult {
    let session = match require_session(&state, &headers, PATH).await {
        Ok(s) => s,
        Err(r) => return Ok(*r),
    };
    if !csrf_ok(&session, &form.csrf) {
        return Ok(csrf_failed());
    }
    console_links::unlink(&state.pool, session.account.id).await?;
    Ok(Redirect::to(PATH).into_response())
}
