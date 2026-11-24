//! The social API under `/api/v1/social` (launchers and game clients, by device token) and the
//! Friends page under `/friends` (signed-in players). The contract is docs/portal-social.md.

use std::sync::Arc;

use askama::Template;
use axum::Json;
use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{DefaultBodyLimit, Form, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{delete, get, post, put};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;
use serde_json::json;
use time::format_description::well_known::Rfc3339;

use crate::db::{self, Session};
use crate::ratelimit::RateLimiter;
use crate::social::{self, AcceptOutcome, BlockOutcome, Peer, RequestOutcome};
use crate::web::{
    AppState, PageResult, Shared, api_error, bearer, csrf_failed, csrf_ok, link_session, message,
    page,
};

/// Presence bodies are a base64 blob of at most 1 KiB plus a little JSON.
const MAX_PRESENCE_BODY: usize = 4096;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/social", get(api_overview))
        .route(
            "/api/v1/social/presence",
            put(api_presence).layer(DefaultBodyLimit::max(MAX_PRESENCE_BODY)),
        )
        .route("/api/v1/social/requests", post(api_request))
        .route("/api/v1/social/requests/{name}", delete(api_cancel))
        .route("/api/v1/social/requests/{name}/accept", post(api_accept))
        .route("/api/v1/social/requests/{name}/decline", post(api_decline))
        .route("/api/v1/social/friends/{name}", delete(api_unfriend))
        .route(
            "/api/v1/social/blocks/{name}",
            put(api_block).delete(api_unblock),
        )
        .route("/api/v1/social/lookup", get(api_lookup))
        .route("/friends", get(friends_page))
        .route("/friends/add", post(friends_add))
        .route("/friends/block", post(friends_block_name))
        .route("/friends/{name}/{action}", post(friends_action))
}

// ---- API ----

/// An early API answer: an error, or a failure already logged. Boxed because responses are large.
struct ApiFail(Box<Response>);

impl From<sqlx::Error> for ApiFail {
    fn from(e: sqlx::Error) -> Self {
        tracing::error!(error = %e, "social request failed");
        Self(Box::new(api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
        )))
    }
}

impl IntoResponse for ApiFail {
    fn into_response(self) -> Response {
        *self.0
    }
}

type ApiResult = Result<Response, ApiFail>;

fn fail(status: StatusCode, error: &str) -> ApiFail {
    ApiFail(Box::new(api_error(status, error)))
}

fn rate_limited() -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(header::RETRY_AFTER, "60")],
        Json(json!({ "error": "rate_limited" })),
    )
        .into_response()
}

/// The caller: a valid device token of a named account that is not banned, within `limiter`.
async fn caller(
    state: &AppState,
    headers: &HeaderMap,
    limiter: &RateLimiter<i32>,
) -> Result<Peer, ApiFail> {
    let account = match bearer(headers) {
        Some(token) => db::device_account(&state.pool, token).await?,
        None => None,
    };
    let Some(account) = account else {
        return Err(fail(StatusCode::UNAUTHORIZED, "invalid_token"));
    };
    if account.is_banned() {
        return Err(fail(StatusCode::FORBIDDEN, "banned"));
    }
    let Some(name) = account.player_name else {
        return Err(fail(StatusCode::CONFLICT, "no_player_name"));
    };
    if !limiter.allow_key(account.id) {
        return Err(ApiFail(Box::new(rate_limited())));
    }
    Ok(Peer {
        id: account.id,
        name,
    })
}

/// The other player named `name`: 404 `unknown_name` when nobody holds it, 400 `self` for
/// the caller.
async fn other(state: &AppState, me: &Peer, name: &str) -> Result<Peer, ApiFail> {
    match social::find(&state.pool, name).await? {
        None => Err(fail(StatusCode::NOT_FOUND, "unknown_name")),
        Some(p) if p.id == me.id => Err(fail(StatusCode::BAD_REQUEST, "self")),
        Some(p) => Ok(p),
    }
}

fn no_content() -> Response {
    StatusCode::NO_CONTENT.into_response()
}

fn rfc3339(t: time::OffsetDateTime) -> String {
    t.format(&Rfc3339).unwrap_or_default()
}

async fn api_overview(State(state): Shared, headers: HeaderMap) -> ApiResult {
    let me = caller(&state, &headers, &state.limits.social_reads).await?;
    let o = social::overview(&state.pool, me.id).await?;
    let friends: Vec<_> = o
        .friends
        .iter()
        .map(|f| {
            let presence = match (&f.presence, f.presence_at) {
                (Some(data), Some(at)) => {
                    json!({ "data_b64": STANDARD.encode(data), "updated_at": rfc3339(at) })
                }
                _ => serde_json::Value::Null,
            };
            json!({ "name": f.name, "online": f.online(), "presence": presence })
        })
        .collect();
    Ok(Json(json!({
        "self": me.name,
        "friends": friends,
        "incoming": o.incoming,
        "outgoing": o.outgoing,
        "blocked": o.blocked,
        "poll_seconds": social::POLL_SECONDS,
    }))
    .into_response())
}

#[derive(Deserialize)]
struct PresenceBody {
    data_b64: Option<String>,
    /// `online` (the default) or `offline`.
    state: Option<String>,
}

async fn api_presence(
    State(state): Shared,
    headers: HeaderMap,
    body: Result<Json<PresenceBody>, JsonRejection>,
) -> ApiResult {
    let body = match body {
        Ok(Json(b)) => b,
        Err(e) if e.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            return Err(fail(StatusCode::PAYLOAD_TOO_LARGE, "too_large"));
        }
        Err(_) => return Err(fail(StatusCode::BAD_REQUEST, "invalid_request")),
    };
    let me = caller(&state, &headers, &state.limits.presence).await?;
    match body.state.as_deref().unwrap_or("online") {
        "offline" => social::clear_presence(&state.pool, me.id).await?,
        "online" => {
            let Some(data) = body.data_b64.and_then(|d| STANDARD.decode(d).ok()) else {
                return Err(fail(StatusCode::BAD_REQUEST, "invalid_request"));
            };
            if data.len() > social::MAX_PRESENCE_BYTES {
                return Err(fail(StatusCode::PAYLOAD_TOO_LARGE, "too_large"));
            }
            social::set_presence(&state.pool, me.id, &data).await?;
        }
        _ => return Err(fail(StatusCode::BAD_REQUEST, "invalid_request")),
    }
    Ok(no_content())
}

#[derive(Deserialize)]
struct NameBody {
    name: String,
}

async fn api_request(
    State(state): Shared,
    headers: HeaderMap,
    body: Result<Json<NameBody>, JsonRejection>,
) -> ApiResult {
    let Ok(Json(body)) = body else {
        return Err(fail(StatusCode::BAD_REQUEST, "invalid_request"));
    };
    let me = caller(&state, &headers, &state.limits.social_changes).await?;
    let them = other(&state, &me, body.name.trim()).await?;
    let status = match social::request(&state.pool, me.id, them.id).await? {
        RequestOutcome::Requested => "requested",
        RequestOutcome::Accepted => "accepted",
        RequestOutcome::AlreadyFriends => {
            return Err(fail(StatusCode::CONFLICT, "already_friends"));
        }
        RequestOutcome::Blocked => return Err(fail(StatusCode::FORBIDDEN, "blocked")),
        RequestOutcome::FriendLimit => return Err(fail(StatusCode::CONFLICT, "friend_limit")),
        RequestOutcome::RequestLimit => return Err(fail(StatusCode::CONFLICT, "request_limit")),
    };
    Ok((
        StatusCode::CREATED,
        Json(json!({ "status": status, "name": them.name })),
    )
        .into_response())
}

async fn api_accept(
    State(state): Shared,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> ApiResult {
    let me = caller(&state, &headers, &state.limits.social_changes).await?;
    let them = other(&state, &me, &name).await?;
    match social::accept(&state.pool, me.id, them.id).await? {
        AcceptOutcome::Accepted => Ok(no_content()),
        AcceptOutcome::NoRequest => Err(fail(StatusCode::NOT_FOUND, "no_request")),
        AcceptOutcome::FriendLimit => Err(fail(StatusCode::CONFLICT, "friend_limit")),
    }
}

async fn api_decline(
    State(state): Shared,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> ApiResult {
    let me = caller(&state, &headers, &state.limits.social_changes).await?;
    let them = other(&state, &me, &name).await?;
    social::withdraw(&state.pool, them.id, me.id).await?;
    Ok(no_content())
}

async fn api_cancel(
    State(state): Shared,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> ApiResult {
    let me = caller(&state, &headers, &state.limits.social_changes).await?;
    let them = other(&state, &me, &name).await?;
    social::withdraw(&state.pool, me.id, them.id).await?;
    Ok(no_content())
}

async fn api_unfriend(
    State(state): Shared,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> ApiResult {
    let me = caller(&state, &headers, &state.limits.social_changes).await?;
    let them = other(&state, &me, &name).await?;
    social::unfriend(&state.pool, me.id, them.id).await?;
    Ok(no_content())
}

async fn api_block(
    State(state): Shared,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> ApiResult {
    let me = caller(&state, &headers, &state.limits.social_changes).await?;
    let them = other(&state, &me, &name).await?;
    match social::block(&state.pool, me.id, them.id).await? {
        BlockOutcome::Blocked => Ok(no_content()),
        BlockOutcome::Limit => Err(fail(StatusCode::CONFLICT, "block_limit")),
    }
}

async fn api_unblock(
    State(state): Shared,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> ApiResult {
    let me = caller(&state, &headers, &state.limits.social_changes).await?;
    let them = other(&state, &me, &name).await?;
    social::unblock(&state.pool, me.id, them.id).await?;
    Ok(no_content())
}

#[derive(Deserialize)]
struct LookupQuery {
    name: Option<String>,
}

async fn api_lookup(
    State(state): Shared,
    headers: HeaderMap,
    query: Result<Query<LookupQuery>, axum::extract::rejection::QueryRejection>,
) -> ApiResult {
    caller(&state, &headers, &state.limits.lookup).await?;
    let name = query.ok().and_then(|Query(q)| q.name).unwrap_or_default();
    match social::find(&state.pool, name.trim()).await? {
        Some(p) => Ok(Json(json!({ "name": p.name, "exists": true })).into_response()),
        None => Err(fail(StatusCode::NOT_FOUND, "unknown_name")),
    }
}

// ---- Friends page ----

struct FriendRow {
    name: String,
    online: bool,
}

#[derive(Template)]
#[template(path = "friends.html")]
struct FriendsPage {
    nav_csrf: Option<String>,
    csrf: String,
    player_name: String,
    friends: Vec<FriendRow>,
    online: usize,
    incoming: Vec<String>,
    outgoing: Vec<String>,
    blocked: Vec<String>,
    /// The outcome of the last form, and whether it went wrong.
    notice: Option<(String, bool)>,
    value: String,
    max_friends: i64,
}

async fn render(
    state: &AppState,
    session: Session,
    status: StatusCode,
    notice: Option<(String, bool)>,
    value: &str,
) -> PageResult {
    let o = social::overview(&state.pool, session.account.id).await?;
    let friends: Vec<FriendRow> = o
        .friends
        .iter()
        .map(|f| FriendRow {
            name: f.name.clone(),
            online: f.online(),
        })
        .collect();
    Ok(page(
        status,
        FriendsPage {
            nav_csrf: Some(session.csrf_token.clone()),
            csrf: session.csrf_token,
            player_name: session.account.player_name.unwrap_or_default(),
            online: friends.iter().filter(|f| f.online).count(),
            friends,
            incoming: o.incoming,
            outgoing: o.outgoing,
            blocked: o.blocked,
            notice,
            value: value.chars().take(32).collect(),
            max_friends: social::MAX_FRIENDS,
        },
    ))
}

macro_rules! guard {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(response) => return Ok(*response),
        }
    };
}

/// A signed-in, named, unbanned player within the per-account limit, for a Friends form.
async fn form_session(
    state: &AppState,
    headers: &HeaderMap,
    csrf: &str,
) -> Result<Session, Box<Response>> {
    let session = link_session(state, headers, "/friends").await?;
    if !csrf_ok(&session, csrf) {
        return Err(Box::new(csrf_failed()));
    }
    if !state.limits.social_changes.allow_key(session.account.id) {
        return Err(Box::new(too_fast()));
    }
    Ok(session)
}

fn too_fast() -> Response {
    message(
        StatusCode::TOO_MANY_REQUESTS,
        "Slow down",
        "Too many friend changes in a minute. Wait a moment, then try again.",
        Some(("/friends", "Friends")),
    )
}

async fn friends_page(State(state): Shared, headers: HeaderMap) -> PageResult {
    let session = guard!(link_session(&state, &headers, "/friends").await);
    if !state.limits.social_reads.allow_key(session.account.id) {
        return Ok(too_fast());
    }
    render(&state, session, StatusCode::OK, None, "").await
}

#[derive(Deserialize)]
struct NameForm {
    csrf: String,
    name: String,
}

/// The named other player for a Friends form, or the page explaining why not.
async fn form_target(
    state: &AppState,
    session: &Session,
    name: &str,
) -> Result<Result<Peer, String>, sqlx::Error> {
    Ok(match social::find(&state.pool, name).await? {
        None => Err(format!("Nobody is called {name}.")),
        Some(p) if p.id == session.account.id => Err("That is you.".to_owned()),
        Some(p) => Ok(p),
    })
}

async fn friends_add(
    State(state): Shared,
    headers: HeaderMap,
    Form(form): Form<NameForm>,
) -> PageResult {
    let session = guard!(form_session(&state, &headers, &form.csrf).await);
    let name: String = form.name.trim().chars().take(32).collect();
    let them = match form_target(&state, &session, &name).await? {
        Ok(p) => p,
        Err(e) => return render(&state, session, StatusCode::OK, Some((e, true)), &name).await,
    };
    let (text, bad) = match social::request(&state.pool, session.account.id, them.id).await? {
        RequestOutcome::Requested => (format!("Friend request sent to {}.", them.name), false),
        RequestOutcome::Accepted => (format!("You and {} are now friends.", them.name), false),
        RequestOutcome::AlreadyFriends => (format!("{} is already your friend.", them.name), true),
        RequestOutcome::Blocked => (
            format!("You block {}. Unblock them first.", them.name),
            true,
        ),
        RequestOutcome::FriendLimit => (
            format!(
                "A friends list holds at most {} players. Remove someone first.",
                social::MAX_FRIENDS
            ),
            true,
        ),
        RequestOutcome::RequestLimit => (
            format!(
                "You have {} requests waiting already. Cancel some first.",
                social::MAX_OUTGOING
            ),
            true,
        ),
    };
    let value = if bad { name.as_str() } else { "" };
    render(&state, session, StatusCode::OK, Some((text, bad)), value).await
}

async fn friends_block_name(
    State(state): Shared,
    headers: HeaderMap,
    Form(form): Form<NameForm>,
) -> PageResult {
    let session = guard!(form_session(&state, &headers, &form.csrf).await);
    let name: String = form.name.trim().chars().take(32).collect();
    let them = match form_target(&state, &session, &name).await? {
        Ok(p) => p,
        Err(e) => return render(&state, session, StatusCode::OK, Some((e, true)), "").await,
    };
    block_and_show(&state, session, them).await
}

async fn block_and_show(state: &AppState, session: Session, them: Peer) -> PageResult {
    let notice = match social::block(&state.pool, session.account.id, them.id).await? {
        BlockOutcome::Blocked => (format!("You block {} now.", them.name), false),
        BlockOutcome::Limit => (
            format!(
                "You can block at most {} players. Unblock someone first.",
                social::MAX_BLOCKS
            ),
            true,
        ),
    };
    render(state, session, StatusCode::OK, Some(notice), "").await
}

#[derive(Deserialize)]
struct CsrfForm {
    csrf: String,
}

async fn friends_action(
    State(state): Shared,
    headers: HeaderMap,
    Path((name, action)): Path<(String, String)>,
    Form(form): Form<CsrfForm>,
) -> PageResult {
    let session = guard!(form_session(&state, &headers, &form.csrf).await);
    let them = match form_target(&state, &session, &name).await? {
        Ok(p) => p,
        Err(e) => return render(&state, session, StatusCode::OK, Some((e, true)), "").await,
    };
    let (me, pool) = (session.account.id, &state.pool);
    match action.as_str() {
        "accept" => match social::accept(pool, me, them.id).await? {
            AcceptOutcome::Accepted => {}
            AcceptOutcome::NoRequest => {
                let text = format!("{} has no request waiting.", them.name);
                return render(&state, session, StatusCode::OK, Some((text, true)), "").await;
            }
            AcceptOutcome::FriendLimit => {
                let text = format!(
                    "A friends list holds at most {} players.",
                    social::MAX_FRIENDS
                );
                return render(&state, session, StatusCode::OK, Some((text, true)), "").await;
            }
        },
        "decline" => {
            social::withdraw(pool, them.id, me).await?;
        }
        "cancel" => {
            social::withdraw(pool, me, them.id).await?;
        }
        "remove" => {
            social::unfriend(pool, me, them.id).await?;
        }
        "block" => return block_and_show(&state, session, them).await,
        "unblock" => {
            social::unblock(pool, me, them.id).await?;
        }
        _ => {
            return Ok(message(
                StatusCode::NOT_FOUND,
                "Not found",
                "There is no page here.",
                Some(("/friends", "Friends")),
            ));
        }
    }
    Ok(Redirect::to("/friends").into_response())
}
