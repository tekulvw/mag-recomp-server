//! HTTP routes: the website (server-rendered, no JavaScript) and the launcher API.

use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration as StdDuration;

use askama::Template;
use axum::extract::rejection::JsonRejection;
use axum::extract::{ConnectInfo, Form, FromRequestParts, MatchedPath, Path, Query, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{AppendHeaders, Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use subtle::ConstantTimeEq;
use time::{Duration, OffsetDateTime};

use crate::config::{Settings, TicketIssuer};
use crate::db::{self, ClaimOutcome, PollOutcome, Session};
use crate::oidc::{FlowSecrets, OidcClient};
use crate::ratelimit::RateLimiter;
use crate::{names, tokens};

pub const SESSION_COOKIE: &str = "__Host-mag_session";
pub const FLOW_COOKIE: &str = "__Host-mag_flow";
const FLOW_TTL_SECONDS: i64 = 600;
const MAX_LABEL_LEN: usize = 64;

pub struct AppState {
    pub pool: PgPool,
    pub oidc: OidcClient,
    pub tickets: TicketIssuer,
    pub settings: Settings,
    pub limits: Limits,
    /// Launcher reports (`PORTAL_REPORTS`); `None` answers `503 reports_disabled`.
    pub reports: Option<Arc<crate::reports::Reports>>,
}

/// Per-address limits on the requests that create rows or carry large bodies, and per-account
/// limits on the social API (docs/portal-social.md).
pub struct Limits {
    pub link_start: RateLimiter,
    pub sign_in: RateLimiter,
    /// Report uploads, before the per-account allowance.
    pub reports: RateLimiter,
    /// `PUT /api/v1/social/presence`, per account.
    pub presence: RateLimiter<i32>,
    /// Friend requests, answers, removals and blocks (API and Friends page), per account.
    pub social_changes: RateLimiter<i32>,
    /// `GET /api/v1/social` and the Friends page, per account.
    pub social_reads: RateLimiter<i32>,
    /// `GET /api/v1/social/lookup`, per account.
    pub lookup: RateLimiter<i32>,
}

impl Default for Limits {
    fn default() -> Self {
        let minute = StdDuration::from_secs(60);
        Self {
            link_start: RateLimiter::new(10, minute),
            sign_in: RateLimiter::new(30, minute),
            reports: RateLimiter::new(60, minute),
            presence: RateLimiter::new(60, minute),
            social_changes: RateLimiter::new(30, minute),
            social_reads: RateLimiter::new(120, minute),
            lookup: RateLimiter::new(30, minute),
        }
    }
}

/// The client's address: the last entry of the configured proxy header, or the peer address.
pub struct ClientIp(pub Option<IpAddr>);

impl FromRequestParts<Arc<AppState>> for ClientIp {
    type Rejection = Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Infallible> {
        if let Some(name) = &state.settings.client_ip_header {
            let ip = parts
                .headers
                .get_all(name.as_str())
                .iter()
                .filter_map(|v| v.to_str().ok())
                .flat_map(|v| v.split(','))
                .next_back()
                .and_then(|v| v.trim().parse().ok());
            return Ok(Self(ip));
        }
        Ok(Self(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|c| c.0.ip()),
        ))
    }
}

pub(crate) type Shared = State<Arc<AppState>>;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(home))
        .route("/privacy", get(privacy))
        .route("/style.css", get(stylesheet))
        .route("/fonts/{name}", get(font))
        .route("/healthz", get(healthz))
        .route("/auth/google", get(sign_in))
        .route("/auth/google/callback", get(sign_in_callback))
        .route("/logout", post(sign_out))
        .route("/account", get(account_page))
        .route("/account/delete", post(delete_account))
        .route("/claim", get(claim_page).post(claim_submit))
        .route("/link", get(link_page).post(link_lookup))
        .route("/link/decide", post(link_decide))
        .route("/devices/{id}/revoke", post(revoke_device))
        .route("/api/v1/device/start", post(api_device_start))
        .route("/api/v1/device/token", post(api_device_token))
        .route("/api/v1/ticket", post(api_ticket))
        .route("/api/v1/ticket-keys", get(api_ticket_keys))
        .route(
            "/api/v1/reports",
            post(api_report).layer(axum::extract::DefaultBodyLimit::max(
                crate::reports::MAX_BODY,
            )),
        )
        .merge(crate::admin_web::routes())
        .merge(crate::social_web::routes())
        .layer(axum::middleware::from_fn(failed_requests))
        .layer(axum::middleware::map_response(security_headers))
        .with_state(state)
}

async fn security_headers(mut response: Response) -> Response {
    let h = response.headers_mut();
    let set = |h: &mut HeaderMap, name: header::HeaderName, value: &'static str| {
        if !h.contains_key(&name) {
            h.insert(name, HeaderValue::from_static(value));
        }
    };
    set(
        h,
        header::CONTENT_SECURITY_POLICY,
        "default-src 'none'; style-src 'self'; img-src 'self'; font-src 'self'; \
         form-action 'self'; frame-ancestors 'none'; base-uri 'none'",
    );
    set(h, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set(h, header::REFERRER_POLICY, "same-origin");
    set(h, header::X_FRAME_OPTIONS, "DENY");
    set(h, header::CACHE_CONTROL, "no-store");
    response
}

// ---- Errors ----

/// Seconds a client waits before retrying after `503 busy` (the `Retry-After` header).
pub const BUSY_RETRY_SECONDS: u64 = 2;

/// A request that failed with an error. Pages get an error page; [`failed_requests`] logs the
/// failure with its route and answers launcher API requests in JSON instead.
pub struct AppError(anyhow::Error);

impl<E: Into<anyhow::Error>> From<E> for AppError {
    fn from(e: E) -> Self {
        Self(e.into())
    }
}

/// The failure behind an [`AppError`] response, for [`failed_requests`].
#[derive(Clone)]
struct Failure {
    error: Arc<str>,
    busy: bool,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let busy = database_unavailable(&self.0);
        let mut response = if busy {
            let mut page = message(
                StatusCode::SERVICE_UNAVAILABLE,
                "Busy",
                "The server cannot reach its database right now. Try again in a moment.",
                None,
            );
            page.headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(BUSY_RETRY_SECONDS));
            page
        } else {
            message(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Something went wrong",
                "The server could not finish that request. Try again in a moment.",
                None,
            )
        };
        response.extensions_mut().insert(Failure {
            error: format!("{:#}", self.0).into(),
            busy,
        });
        response
    }
}

/// Whether `e` (or anything it wraps) says the database cannot be used for a moment: the pool
/// had no connection in time, the connection broke, or the server is shutting down, starting,
/// or read-only while it fails over. Such requests answer `503 busy`, not `500`.
pub fn database_unavailable(e: &anyhow::Error) -> bool {
    e.chain().any(|cause| {
        cause
            .downcast_ref::<sqlx::Error>()
            .is_some_and(sqlx_unavailable)
    })
}

/// [`database_unavailable`] for one sqlx error.
pub fn sqlx_unavailable(e: &sqlx::Error) -> bool {
    match e {
        sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed | sqlx::Error::Io(_) => true,
        sqlx::Error::Database(db) => db.code().is_some_and(|code| unavailable_sqlstate(&code)),
        _ => false,
    }
}

/// PostgreSQL error codes for a database that is unavailable rather than refusing the request:
/// class 08 (connection exception), 57P01 to 57P03 (shutting down, crashed, cannot connect
/// now) and 25006 (read-only transaction: a standby, or a primary during failover).
fn unavailable_sqlstate(code: &str) -> bool {
    code.starts_with("08") || matches!(code, "57P01" | "57P02" | "57P03" | "25006")
}

/// `503 {"error":"busy"}` with `Retry-After`: the launcher API's answer while the database is
/// unavailable.
pub(crate) fn api_busy() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::RETRY_AFTER, HeaderValue::from(BUSY_RETRY_SECONDS))],
        Json(json!({ "error": "busy" })),
    )
        .into_response()
}

/// Logs every request that failed with an [`AppError`], with its method and route, and gives
/// launcher API requests (`/api/…`) a JSON error in place of the error page: `503 busy` (with
/// `Retry-After`) while the database is unavailable, `500 server_error` otherwise.
async fn failed_requests(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let method = request.method().clone();
    // The route pattern, not the path, so names and codes in paths stay out of the log.
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_else(|| request.uri().path().to_owned());
    let api = request.uri().path().starts_with("/api/");
    let response = next.run(request).await;
    let Some(failure) = response.extensions().get::<Failure>().cloned() else {
        return response;
    };
    let status = response.status().as_u16();
    tracing::error!(%method, %route, status, error = %failure.error, "request failed");
    match (api, failure.busy) {
        (false, _) => response,
        (true, true) => api_busy(),
        (true, false) => api_error(StatusCode::INTERNAL_SERVER_ERROR, "server_error"),
    }
}

pub(crate) type PageResult = Result<Response, AppError>;

// ---- Templates ----

#[derive(Template)]
#[template(path = "home.html")]
struct HomePage {
    nav_csrf: Option<String>,
    player_name: Option<String>,
}

#[derive(Template)]
#[template(path = "privacy.html")]
struct PrivacyPage {
    nav_csrf: Option<String>,
    contact: Option<String>,
    report_days: i64,
    presence_seconds: i64,
}

#[derive(Template)]
#[template(path = "message.html")]
struct MessagePage {
    nav_csrf: Option<String>,
    title: String,
    body: String,
    link: Option<(String, String)>,
}

struct DeviceRow {
    id: i32,
    label: String,
    created: String,
    last_used: String,
}

#[derive(Template)]
#[template(path = "account.html")]
struct AccountPage {
    nav_csrf: Option<String>,
    csrf: String,
    player_name: Option<String>,
    email: Option<String>,
    ban_reason: Option<String>,
    banned: bool,
    devices: Vec<DeviceRow>,
    staff_role: Option<&'static str>,
}

#[derive(Template)]
#[template(path = "claim.html")]
struct ClaimPage {
    nav_csrf: Option<String>,
    csrf: String,
    next: String,
    value: String,
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "link.html")]
struct LinkPage {
    nav_csrf: Option<String>,
    csrf: String,
    code: String,
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "link_confirm.html")]
struct LinkConfirmPage {
    nav_csrf: Option<String>,
    csrf: String,
    code: String,
    shown_code: String,
    label: String,
    requested: String,
    player_name: String,
}

pub(crate) fn page<T: Template>(status: StatusCode, template: T) -> Response {
    match template.render() {
        Ok(html) => (status, Html(html)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "template failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "template error").into_response()
        }
    }
}

pub(crate) fn message(
    status: StatusCode,
    title: &str,
    body: &str,
    link: Option<(&str, &str)>,
) -> Response {
    page(
        status,
        MessagePage {
            nav_csrf: None,
            title: title.to_owned(),
            body: body.to_owned(),
            link: link.map(|(href, text)| (href.to_owned(), text.to_owned())),
        },
    )
}

pub(crate) fn when(t: OffsetDateTime) -> String {
    let format = time::macros::format_description!("[year]-[month]-[day] [hour]:[minute] UTC");
    t.format(&format).unwrap_or_default()
}

// ---- Cookies and sessions ----

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

fn set_cookie(name: &str, value: &str, max_age: i64) -> (header::HeaderName, String) {
    (
        header::SET_COOKIE,
        format!("{name}={value}; Max-Age={max_age}; Path=/; Secure; HttpOnly; SameSite=Lax"),
    )
}

pub(crate) async fn current_session(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<Session>, AppError> {
    match cookie(headers, SESSION_COOKIE) {
        Some(value) => Ok(db::session(&state.pool, value).await?),
        None => Ok(None),
    }
}

/// A local path to return to after sign-in; anything else becomes `/account`.
fn safe_next(next: Option<&str>) -> String {
    match next {
        Some(n)
            // Printable ASCII only: browsers drop tabs and newlines, so "/\t/host" means "//host".
            if n.starts_with('/')
                && !n.starts_with("//")
                && n.len() <= 512
                && n.bytes().all(|b| b.is_ascii_graphic() && b != b'\\') =>
        {
            n.to_owned()
        }
        _ => "/account".to_owned(),
    }
}

fn sign_in_redirect(next: &str) -> Response {
    Redirect::to(&format!(
        "/auth/google?next={}",
        crate::oidc::form_urlencode(next)
    ))
    .into_response()
}

pub(crate) fn csrf_ok(session: &Session, submitted: &str) -> bool {
    session
        .csrf_token
        .as_bytes()
        .ct_eq(submitted.as_bytes())
        .into()
}

pub(crate) fn csrf_failed() -> Response {
    message(
        StatusCode::FORBIDDEN,
        "Form expired",
        "That form was out of date. Go back, reload the page and try again.",
        Some(("/account", "Your account")),
    )
}

/// The signed-in session, or a response sending the visitor to sign in.
pub(crate) async fn require_session(
    state: &AppState,
    headers: &HeaderMap,
    next: &str,
) -> Result<Session, Box<Response>> {
    match current_session(state, headers).await {
        Ok(Some(s)) => Ok(s),
        Ok(None) => Err(Box::new(sign_in_redirect(next))),
        Err(e) => Err(Box::new(e.into_response())),
    }
}

// ---- Public pages ----

async fn home(State(state): Shared, headers: HeaderMap) -> PageResult {
    let session = current_session(&state, &headers).await?;
    Ok(page(
        StatusCode::OK,
        HomePage {
            player_name: session.as_ref().and_then(|s| s.account.player_name.clone()),
            nav_csrf: session.map(|s| s.csrf_token),
        },
    ))
}

async fn privacy(State(state): Shared, headers: HeaderMap) -> PageResult {
    let nav_csrf = current_session(&state, &headers)
        .await?
        .map(|s| s.csrf_token);
    Ok(page(
        StatusCode::OK,
        PrivacyPage {
            nav_csrf,
            contact: state.settings.operator_contact.clone(),
            report_days: state.reports.as_ref().map_or(30, |r| r.retention_days),
            presence_seconds: crate::social::PRESENCE_TTL_SECONDS,
        },
    ))
}

/// A web font from the configured folder. Names are lowercase letters, digits and `-` ending
/// `.woff2`, so a request can never leave the folder.
async fn font(State(state): Shared, Path(name): Path<String>) -> Response {
    let plain = name.len() <= 80
        && name.ends_with(".woff2")
        && name[..name.len() - 6]
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    let Some(dir) = state.settings.font_dir.as_ref().filter(|_| plain) else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    match tokio::fs::read(dir.join(&name)).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "font/woff2"),
                (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

const STYLESHEET: &str = include_str!("../static/style.css");

/// A short digest of the stylesheet. Pages link `/style.css?v=<this>`, so a new stylesheet has a
/// new address that no browser or CDN (Cloudflare) has cached, and the old one can be cached long.
pub fn style_version() -> &'static str {
    static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VERSION.get_or_init(|| tokens::b64url(&tokens::sha256(STYLESHEET.as_bytes())[..9]))
}

#[derive(Deserialize)]
struct StyleQuery {
    v: Option<String>,
}

async fn stylesheet(Query(q): Query<StyleQuery>) -> Response {
    // Only the current versioned address may be cached for good; anything else could be stale.
    let cache = if q.v.as_deref() == Some(style_version()) {
        "public, max-age=31536000, immutable"
    } else {
        "public, max-age=300"
    };
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, cache),
        ],
        STYLESHEET,
    )
        .into_response()
}

async fn healthz(State(state): Shared) -> Response {
    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => (StatusCode::OK, "ok").into_response(),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "database unavailable").into_response(),
    }
}

// ---- Sign-in ----

#[derive(Deserialize)]
struct NextQuery {
    next: Option<String>,
}

async fn sign_in(
    State(state): Shared,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    Query(q): Query<NextQuery>,
) -> PageResult {
    let next = safe_next(q.next.as_deref());
    if current_session(&state, &headers).await?.is_some() {
        return Ok(Redirect::to(&next).into_response());
    }
    if !state.limits.sign_in.allow(ip) {
        return Ok(message(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many sign-in attempts",
            "Wait a minute, then try again.",
            None,
        ));
    }
    let flow = FlowSecrets::new();
    db::create_login_flow(
        &state.pool,
        &db::LoginFlow {
            state: flow.state.clone(),
            nonce: flow.nonce.clone(),
            pkce_verifier: flow.pkce_verifier.clone(),
            return_to: next,
        },
        Duration::seconds(FLOW_TTL_SECONDS),
    )
    .await?;
    Ok((
        AppendHeaders([set_cookie(FLOW_COOKIE, &flow.state, FLOW_TTL_SECONDS)]),
        Redirect::to(&state.oidc.authorize_url(&flow)),
    )
        .into_response())
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

async fn sign_in_callback(
    State(state): Shared,
    headers: HeaderMap,
    Query(q): Query<CallbackQuery>,
) -> PageResult {
    let clear_flow = set_cookie(FLOW_COOKIE, "", 0);
    let retry = Some(("/auth/google", "Try again"));
    if q.error.is_some() {
        return Ok((
            AppendHeaders([clear_flow]),
            message(
                StatusCode::OK,
                "Sign-in cancelled",
                "Google sign-in did not finish.",
                retry,
            ),
        )
            .into_response());
    }
    let (Some(code), Some(flow_state)) = (q.code, q.state) else {
        return Ok(message(
            StatusCode::BAD_REQUEST,
            "Sign-in failed",
            "The sign-in reply was incomplete.",
            retry,
        ));
    };
    // The state must come back to the same browser that started the flow.
    let same_browser = cookie(&headers, FLOW_COOKIE)
        .is_some_and(|c| bool::from(c.as_bytes().ct_eq(flow_state.as_bytes())));
    if !same_browser {
        return Ok(message(
            StatusCode::BAD_REQUEST,
            "Sign-in failed",
            "This sign-in was started in another browser or has expired.",
            retry,
        ));
    }
    let Some(flow) = db::take_login_flow(&state.pool, &flow_state).await? else {
        return Ok(message(
            StatusCode::BAD_REQUEST,
            "Sign-in expired",
            "That sign-in took too long.",
            retry,
        ));
    };
    let identity = match state
        .oidc
        .complete(&code, &flow.pkce_verifier, &flow.nonce)
        .await
    {
        Ok(identity) => identity,
        Err(e) => {
            tracing::warn!(error = %e, "sign-in rejected");
            return Ok(message(
                StatusCode::BAD_GATEWAY,
                "Sign-in failed",
                "Google sign-in could not be verified.",
                retry,
            ));
        }
    };
    let account = db::upsert_identity(
        &state.pool,
        state.oidc.provider(),
        &identity.subject,
        identity.email.as_deref(),
    )
    .await?;
    let session = db::create_session(
        &state.pool,
        account.id,
        Duration::days(state.settings.session_days),
    )
    .await?;
    tracing::info!(account = account.id, "signed in");
    let max_age = state.settings.session_days * 86_400;
    Ok((
        AppendHeaders([
            clear_flow,
            set_cookie(SESSION_COOKIE, &session.cookie, max_age),
        ]),
        Redirect::to(&flow.return_to),
    )
        .into_response())
}

#[derive(Deserialize)]
struct CsrfForm {
    csrf: String,
}

async fn sign_out(
    State(state): Shared,
    headers: HeaderMap,
    Form(form): Form<CsrfForm>,
) -> PageResult {
    if let Some(session) = current_session(&state, &headers).await? {
        if !csrf_ok(&session, &form.csrf) {
            return Ok(csrf_failed());
        }
        db::delete_session(&state.pool, &session.id_hash).await?;
    }
    Ok((
        AppendHeaders([set_cookie(SESSION_COOKIE, "", 0)]),
        Redirect::to("/"),
    )
        .into_response())
}

// ---- Account ----

async fn account_page(State(state): Shared, headers: HeaderMap) -> PageResult {
    let session = match require_session(&state, &headers, "/account").await {
        Ok(s) => s,
        Err(r) => return Ok(*r),
    };
    let devices = db::devices(&state.pool, session.account.id)
        .await?
        .into_iter()
        .map(|d| DeviceRow {
            id: d.id,
            label: d.label,
            created: when(d.created_at),
            last_used: d.last_used_at.map(when).unwrap_or_else(|| "never".into()),
        })
        .collect();
    let staff_role = crate::staff::role_for_email(
        &state.pool,
        &state.settings.owner_emails,
        session.account.email.as_deref(),
    )
    .await?
    .map(crate::staff::Role::as_str);
    let a = session.account;
    Ok(page(
        StatusCode::OK,
        AccountPage {
            nav_csrf: Some(session.csrf_token.clone()),
            csrf: session.csrf_token,
            banned: a.is_banned(),
            player_name: a.player_name,
            email: a.email,
            ban_reason: a.ban_reason,
            devices,
            staff_role,
        },
    ))
}

#[derive(Deserialize)]
struct DeleteForm {
    csrf: String,
    confirm: String,
}

async fn delete_account(
    State(state): Shared,
    headers: HeaderMap,
    Form(form): Form<DeleteForm>,
) -> PageResult {
    let session = match require_session(&state, &headers, "/account").await {
        Ok(s) => s,
        Err(r) => return Ok(*r),
    };
    if !csrf_ok(&session, &form.csrf) {
        return Ok(csrf_failed());
    }
    // Deleting and signing up again would otherwise shed a ban.
    if session.account.is_banned() {
        return Ok(message(
            StatusCode::FORBIDDEN,
            "Account kept",
            "Banned accounts cannot be deleted here. Ask the operator to delete it.",
            Some(("/account", "Back to your account")),
        ));
    }
    let expected = session.account.player_name.as_deref().unwrap_or("delete");
    if !form.confirm.trim().eq_ignore_ascii_case(expected) {
        return Ok(message(
            StatusCode::BAD_REQUEST,
            "Account kept",
            "The confirmation did not match, so nothing was deleted.",
            Some(("/account", "Back to your account")),
        ));
    }
    if let Some(reports) = &state.reports {
        crate::reports::delete_account_logs(
            &state.pool,
            reports.store.as_ref(),
            session.account.id,
        )
        .await;
    }
    db::delete_account(&state.pool, session.account.id).await?;
    tracing::info!(account = session.account.id, "account deleted");
    Ok((
        AppendHeaders([set_cookie(SESSION_COOKIE, "", 0)]),
        message(
            StatusCode::OK,
            "Account deleted",
            "Your account, sign-in and linked launchers are gone. Your player name stays reserved so nobody else can take over its characters.",
            Some(("/", "Home")),
        ),
    )
        .into_response())
}

#[derive(Deserialize)]
struct RevokeForm {
    csrf: String,
}

async fn revoke_device(
    State(state): Shared,
    headers: HeaderMap,
    Path(id): Path<i32>,
    Form(form): Form<RevokeForm>,
) -> PageResult {
    let session = match require_session(&state, &headers, "/account").await {
        Ok(s) => s,
        Err(r) => return Ok(*r),
    };
    if !csrf_ok(&session, &form.csrf) {
        return Ok(csrf_failed());
    }
    db::revoke_device(&state.pool, session.account.id, id).await?;
    Ok(Redirect::to("/account").into_response())
}

// ---- Player name ----

async fn claim_page(
    State(state): Shared,
    headers: HeaderMap,
    Query(q): Query<NextQuery>,
) -> PageResult {
    let next = safe_next(q.next.as_deref());
    let session = match require_session(&state, &headers, "/claim").await {
        Ok(s) => s,
        Err(r) => return Ok(*r),
    };
    if session.account.player_name.is_some() {
        return Ok(Redirect::to(&next).into_response());
    }
    Ok(page(
        StatusCode::OK,
        ClaimPage {
            nav_csrf: Some(session.csrf_token.clone()),
            csrf: session.csrf_token,
            next,
            value: String::new(),
            error: None,
        },
    ))
}

#[derive(Deserialize)]
struct ClaimForm {
    csrf: String,
    name: String,
    next: Option<String>,
}

async fn claim_submit(
    State(state): Shared,
    headers: HeaderMap,
    Form(form): Form<ClaimForm>,
) -> PageResult {
    let next = safe_next(form.next.as_deref());
    let session = match require_session(&state, &headers, "/claim").await {
        Ok(s) => s,
        Err(r) => return Ok(*r),
    };
    if !csrf_ok(&session, &form.csrf) {
        return Ok(csrf_failed());
    }
    let name = form.name.trim();
    let error = match db::claim_name(&state.pool, session.account.id, name).await? {
        ClaimOutcome::Claimed | ClaimOutcome::AlreadyNamed => {
            return Ok(Redirect::to(&next).into_response());
        }
        ClaimOutcome::Invalid => "Use 3 to 15 letters, digits, - or _, starting with a letter.",
        ClaimOutcome::Taken => "Someone already has that name.",
        ClaimOutcome::Reserved => {
            "That name is reserved. If it was yours on the server before, ask the operator to assign it to you."
        }
    };
    Ok(page(
        StatusCode::OK,
        ClaimPage {
            nav_csrf: Some(session.csrf_token.clone()),
            csrf: session.csrf_token,
            next,
            value: name.chars().take(32).collect(),
            error: Some(error.to_owned()),
        },
    ))
}

// ---- Launcher linking (browser side) ----

#[derive(Deserialize)]
struct CodeQuery {
    code: Option<String>,
}

/// Signed in, named and not banned; otherwise the page to show instead.
pub(crate) async fn link_session(
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
            "This account is banned, so it cannot link a launcher or use friends.",
            Some(("/account", "Your account")),
        )));
    }
    Ok(session)
}

async fn link_page(
    State(state): Shared,
    headers: HeaderMap,
    Query(q): Query<CodeQuery>,
) -> PageResult {
    let code = q.code.unwrap_or_default();
    let next = if code.is_empty() {
        "/link".to_owned()
    } else {
        format!("/link?code={}", crate::oidc::form_urlencode(&code))
    };
    let session = match link_session(&state, &headers, &next).await {
        Ok(s) => s,
        Err(r) => return Ok(*r),
    };
    Ok(page(
        StatusCode::OK,
        LinkPage {
            nav_csrf: Some(session.csrf_token.clone()),
            csrf: session.csrf_token,
            code: code.chars().take(16).collect(),
            error: None,
        },
    ))
}

#[derive(Deserialize)]
struct LinkForm {
    csrf: String,
    code: String,
}

async fn link_lookup(
    State(state): Shared,
    headers: HeaderMap,
    Form(form): Form<LinkForm>,
) -> PageResult {
    let session = match link_session(&state, &headers, "/link").await {
        Ok(s) => s,
        Err(r) => return Ok(*r),
    };
    if !csrf_ok(&session, &form.csrf) {
        return Ok(csrf_failed());
    }
    let pending = match tokens::normalize_user_code(&form.code) {
        Some(code) => db::pending_link(&state.pool, &code).await?,
        None => None,
    };
    let Some(pending) = pending else {
        return Ok(page(
            StatusCode::OK,
            LinkPage {
                nav_csrf: Some(session.csrf_token.clone()),
                csrf: session.csrf_token,
                code: form.code.chars().take(16).collect(),
                error: Some("That code is not waiting to be linked. Check it, or start again in the launcher.".into()),
            },
        ));
    };
    Ok(page(
        StatusCode::OK,
        LinkConfirmPage {
            nav_csrf: Some(session.csrf_token.clone()),
            csrf: session.csrf_token,
            shown_code: tokens::format_user_code(&pending.user_code),
            code: pending.user_code,
            label: pending.client_label,
            requested: when(pending.created_at),
            player_name: session.account.player_name.unwrap_or_default(),
        },
    ))
}

#[derive(Deserialize)]
struct DecideForm {
    csrf: String,
    code: String,
    decision: String,
}

async fn link_decide(
    State(state): Shared,
    headers: HeaderMap,
    Form(form): Form<DecideForm>,
) -> PageResult {
    let session = match link_session(&state, &headers, "/link").await {
        Ok(s) => s,
        Err(r) => return Ok(*r),
    };
    if !csrf_ok(&session, &form.csrf) {
        return Ok(csrf_failed());
    }
    let approve = form.decision == "approve";
    let decided = match tokens::normalize_user_code(&form.code) {
        Some(code) => db::decide_link(&state.pool, &code, session.account.id, approve).await?,
        None => false,
    };
    let back = Some(("/account", "Your account"));
    Ok(match (decided, approve) {
        (false, _) => message(
            StatusCode::OK,
            "Code expired",
            "That code is no longer waiting. Start again in the launcher.",
            back,
        ),
        (true, true) => message(
            StatusCode::OK,
            "Launcher linked",
            "Return to the launcher; it signs in by itself within a few seconds.",
            back,
        ),
        (true, false) => message(
            StatusCode::OK,
            "Link refused",
            "The launcher was not linked.",
            back,
        ),
    })
}

// ---- Launcher API ----

pub(crate) fn api_error(status: StatusCode, error: &str) -> Response {
    (status, Json(json!({ "error": error }))).into_response()
}

#[derive(Deserialize, Default)]
struct StartRequest {
    client: Option<String>,
}

#[derive(Serialize)]
struct StartResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: String,
    expires_in: i64,
    interval: i64,
}

/// A short description of the launcher, shown when the player approves it. Printable ASCII
/// only, so direction overrides and look-alike letters cannot disguise it.
fn client_label(raw: Option<&str>) -> String {
    let label: String = raw
        .unwrap_or("")
        .chars()
        .filter(|c| *c == ' ' || c.is_ascii_graphic())
        .take(MAX_LABEL_LEN)
        .collect();
    let label = label.trim();
    if label.is_empty() {
        "Unnamed launcher".to_owned()
    } else {
        label.to_owned()
    }
}

async fn api_device_start(
    State(state): Shared,
    ClientIp(ip): ClientIp,
    body: Result<Json<StartRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    let request = match body {
        Ok(Json(b)) => b,
        // No body at all is fine; a body that is not the expected JSON is not.
        Err(JsonRejection::MissingJsonContentType(_)) => StartRequest::default(),
        Err(_) => return Ok(api_error(StatusCode::BAD_REQUEST, "invalid_request")),
    };
    if !state.limits.link_start.allow(ip) {
        return Ok(api_error(StatusCode::TOO_MANY_REQUESTS, "rate_limited"));
    }
    let label = client_label(request.client.as_deref());
    let s = &state.settings;
    let Some(code) = db::create_device_code(
        &state.pool,
        &label,
        Duration::seconds(s.link_ttl_seconds),
        s.max_pending_links,
    )
    .await?
    else {
        return Ok(api_error(StatusCode::SERVICE_UNAVAILABLE, "busy"));
    };
    let shown = tokens::format_user_code(&code.user_code);
    let verification_uri = format!("{}/link", s.public_url);
    Ok(Json(StartResponse {
        verification_uri_complete: format!("{verification_uri}?code={shown}"),
        verification_uri,
        device_code: code.device_code,
        user_code: shown,
        expires_in: s.link_ttl_seconds,
        interval: s.link_poll_seconds,
    })
    .into_response())
}

#[derive(Deserialize)]
struct TokenRequest {
    device_code: String,
}

async fn api_device_token(
    State(state): Shared,
    body: Result<Json<TokenRequest>, JsonRejection>,
) -> Result<Response, AppError> {
    let Ok(Json(request)) = body else {
        return Ok(api_error(StatusCode::BAD_REQUEST, "invalid_request"));
    };
    let outcome = db::poll_device_code(
        &state.pool,
        &request.device_code,
        Duration::seconds(state.settings.link_poll_seconds),
    )
    .await?;
    let bad = StatusCode::BAD_REQUEST;
    Ok(match outcome {
        PollOutcome::Pending => api_error(bad, "authorization_pending"),
        PollOutcome::SlowDown => api_error(bad, "slow_down"),
        PollOutcome::Denied => api_error(bad, "access_denied"),
        PollOutcome::Expired => api_error(bad, "expired_token"),
        PollOutcome::Invalid => api_error(bad, "invalid_grant"),
        PollOutcome::Linked {
            device_token,
            player_name,
        } => Json(json!({ "device_token": device_token, "player_name": player_name }))
            .into_response(),
    })
}

pub(crate) fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

#[derive(Deserialize, Default)]
struct TicketRequest {
    /// The game server the ticket is for; the default audience when absent or empty.
    audience: Option<String>,
}

/// The optional ticket request body: nothing (or only whitespace) asks for the default.
fn ticket_request(body: &[u8]) -> Option<TicketRequest> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Some(TicketRequest::default());
    }
    serde_json::from_slice(body).ok()
}

async fn api_ticket(
    State(state): Shared,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    let Some(request) = ticket_request(&body) else {
        return Ok(api_error(StatusCode::BAD_REQUEST, "invalid_request"));
    };
    let account = match bearer(&headers) {
        Some(token) => db::device_account(&state.pool, token).await?,
        None => None,
    };
    let Some(account) = account else {
        return Ok(api_error(StatusCode::UNAUTHORIZED, "invalid_token"));
    };
    let Some(audience) = state.tickets.resolve_audience(request.audience.as_deref()) else {
        return Ok(api_error(StatusCode::BAD_REQUEST, "unknown_audience"));
    };
    if account.is_banned() {
        return Ok((
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "banned", "reason": account.ban_reason })),
        )
            .into_response());
    }
    let Some(name) = account
        .player_name
        .as_deref()
        .filter(|n| names::valid_player_name(n))
    else {
        return Ok(api_error(StatusCode::CONFLICT, "no_player_name"));
    };
    let now = OffsetDateTime::now_utc().unix_timestamp() as u64;
    let account_id = u32::try_from(account.id)?;
    let (ticket, expires_at) = state.tickets.issue(audience, account_id, name, now)?;
    Ok(Json(json!({
        "ticket": STANDARD.encode(ticket),
        "expires_at": expires_at,
        "account_id": account_id,
        "player_name": name,
        "audience": audience,
    }))
    .into_response())
}

async fn api_report(
    State(state): Shared,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    body: Result<axum::body::Bytes, axum::extract::rejection::BytesRejection>,
) -> Result<Response, AppError> {
    use crate::reports::{self, SubmitOutcome};
    let body = match body {
        Ok(b) => b,
        Err(e) if e.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            return Ok(api_error(StatusCode::PAYLOAD_TOO_LARGE, "too_large"));
        }
        Err(_) => {
            return Ok(invalid_report("the body could not be read"));
        }
    };
    let Some(service) = state.reports.clone() else {
        return Ok(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "reports_disabled",
        ));
    };
    if !state.limits.reports.allow(ip) {
        return Ok((
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, "60")],
            Json(json!({ "error": "rate_limited" })),
        )
            .into_response());
    }
    let account = match bearer(&headers) {
        Some(token) => db::device_account(&state.pool, token).await?,
        None => None,
    };
    let Some(account) = account else {
        return Ok(api_error(StatusCode::UNAUTHORIZED, "invalid_token"));
    };
    if account.is_banned() {
        return Ok(api_error(StatusCode::FORBIDDEN, "banned"));
    }
    let report = match reports::parse(&body) {
        Ok(r) => r,
        Err(reports::Invalid(detail)) => return Ok(invalid_report(&detail)),
    };
    let id = report.id.clone();
    let received = |status: StatusCode, at: OffsetDateTime| {
        let at = at
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();
        (status, Json(json!({ "report_id": id, "received_at": at }))).into_response()
    };
    let outcome = match reports::submit(&state.pool, &service, &account, report).await {
        Ok(outcome) => outcome,
        Err(e) => {
            let busy = database_unavailable(&e);
            tracing::error!(error = %format!("{e:#}"), report = %id, busy, "report not stored");
            // A launcher drops a report after a few 500s (it blames the report), so a database
            // that is only unavailable answers 503 busy.
            return Ok(if busy {
                api_busy()
            } else {
                api_error(StatusCode::INTERNAL_SERVER_ERROR, "server_error")
            });
        }
    };
    Ok(match outcome {
        SubmitOutcome::Created(at) => {
            tracing::info!(account = account.id, report = %id, "report received");
            received(StatusCode::CREATED, at)
        }
        SubmitOutcome::Duplicate(at) => received(StatusCode::OK, at),
        SubmitOutcome::Conflict => api_error(StatusCode::CONFLICT, "report_id_conflict"),
        SubmitOutcome::RateLimited(seconds) => (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, seconds.to_string())],
            Json(json!({ "error": "rate_limited" })),
        )
            .into_response(),
    })
}

fn invalid_report(detail: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": "invalid_report", "detail": detail })),
    )
        .into_response()
}

async fn api_ticket_keys(State(state): Shared) -> Response {
    (
        [(header::CACHE_CONTROL, "public, max-age=300")],
        Json(json!({
            "keys": [{
                "id": state.tickets.key_id,
                "alg": "ES256",
                "spki": STANDARD.encode(state.tickets.public_key_der()),
            }]
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_paths() {
        assert_eq!(safe_next(Some("/link?code=AB")), "/link?code=AB");
        for bad in [
            "//evil.example",
            "https://evil.example",
            "/\\evil",
            "/\t/evil.example",
            "/\n/evil.example",
            "/ /evil",
            "/\u{7f}",
            "/é",
            "",
            "x",
        ] {
            assert_eq!(safe_next(Some(bad)), "/account", "{bad}");
        }
        assert_eq!(safe_next(None), "/account");
    }

    #[test]
    fn labels() {
        assert_eq!(client_label(None), "Unnamed launcher");
        assert_eq!(client_label(Some(" MAG \u{7} launcher ")), "MAG  launcher");
        assert_eq!(client_label(Some("evil\u{202e}rehcnual")), "evilrehcnual");
        assert_eq!(client_label(Some(&"x".repeat(100))).len(), MAX_LABEL_LEN);
    }

    #[test]
    fn ticket_bodies() {
        for empty in [&b""[..], b"  \n", b"{}", b"{\"audience\":null}"] {
            assert_eq!(ticket_request(empty).unwrap().audience, None);
        }
        assert_eq!(
            ticket_request(br#"{"audience":"Test.example.test"}"#)
                .unwrap()
                .audience
                .as_deref(),
            Some("Test.example.test")
        );
        for bad in [&b"nope"[..], b"[]", b"{\"audience\":7}"] {
            assert!(ticket_request(bad).is_none());
        }
    }

    #[test]
    fn unavailable_databases() {
        for e in [
            sqlx::Error::PoolTimedOut,
            sqlx::Error::PoolClosed,
            sqlx::Error::Io(std::io::ErrorKind::ConnectionReset.into()),
        ] {
            assert!(sqlx_unavailable(&e), "{e}");
            let wrapped = anyhow::Error::from(e).context("loading the account");
            assert!(database_unavailable(&wrapped));
        }
        for e in [
            sqlx::Error::RowNotFound,
            sqlx::Error::Protocol("nope".into()),
        ] {
            assert!(!sqlx_unavailable(&e), "{e}");
        }
        assert!(!database_unavailable(&anyhow::anyhow!("boom")));
        for code in [
            "08000", "08006", "08P01", "57P01", "57P02", "57P03", "25006",
        ] {
            assert!(unavailable_sqlstate(code), "{code}");
        }
        for code in ["23505", "42P01", "40001", "57014", "25P02", "53300"] {
            assert!(!unavailable_sqlstate(code), "{code}");
        }
    }

    /// Log lines written while the guard lives, on this thread.
    #[derive(Clone, Default)]
    struct Captured(Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Captured {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
        type Writer = Captured;

        fn make_writer(&'a self) -> Captured {
            self.clone()
        }
    }

    #[tokio::test]
    async fn failed_requests_are_logged_and_answered() {
        let log = Captured::default();
        let _guard = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_writer(log.clone())
                .with_ansi(false)
                .finish(),
        );
        async fn busy() -> Result<Response, AppError> {
            Err(sqlx::Error::PoolTimedOut.into())
        }
        async fn broken() -> Result<Response, AppError> {
            Err(anyhow::anyhow!("boom").into())
        }
        let app = Router::new()
            .route("/api/v1/things/{name}", post(busy))
            .route("/api/v1/broken", get(broken))
            .route("/pages/{name}", get(busy))
            .route("/pages/broken", get(broken))
            .layer(axum::middleware::from_fn(failed_requests));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let http = reqwest::Client::new();

        let r = http
            .post(format!("{base}/api/v1/things/Alpha"))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(r.headers()[header::RETRY_AFTER], "2");
        assert_eq!(r.text().await.unwrap(), r#"{"error":"busy"}"#);

        let r = http
            .get(format!("{base}/api/v1/broken"))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(r.text().await.unwrap(), r#"{"error":"server_error"}"#);

        let r = http
            .get(format!("{base}/pages/Alpha"))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(r.headers()[header::RETRY_AFTER], "2");
        assert!(r.text().await.unwrap().contains("Busy"));

        let r = http
            .get(format!("{base}/pages/broken"))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(r.headers().get(header::RETRY_AFTER).is_none());
        assert!(r.text().await.unwrap().contains("Something went wrong"));

        let log = String::from_utf8(log.0.lock().unwrap().clone()).unwrap();
        let lines: Vec<&str> = log
            .lines()
            .filter(|l| l.contains("request failed"))
            .collect();
        assert_eq!(lines.len(), 4, "{log}");
        assert!(
            lines[0].contains("method=POST") && lines[0].contains("route=/api/v1/things/{name}"),
            "{log}"
        );
        assert!(lines[0].contains("status=503") && lines[0].contains("pool timed out"));
        assert!(lines[1].contains("method=GET") && lines[1].contains("route=/api/v1/broken"));
        assert!(lines[1].contains("status=500") && lines[1].contains("boom"));
        assert!(lines[2].contains("route=/pages/{name}"));
        assert!(lines[3].contains("route=/pages/broken"));
        // Routes, not paths: names in paths stay out of the log.
        assert!(!log.contains("Alpha"), "{log}");
    }

    #[test]
    fn cookies() {
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            HeaderValue::from_static("a=1; __Host-mag_session=mgs_x; b=2"),
        );
        assert_eq!(cookie(&h, SESSION_COOKIE), Some("mgs_x"));
        assert_eq!(cookie(&h, FLOW_COOKIE), None);
    }
}
