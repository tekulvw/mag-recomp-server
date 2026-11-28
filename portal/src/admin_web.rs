//! The staff admin panel under `/admin`: users, reserved names, staff roles, an audit log and
//! the game server's characters (`game`), service accounts (`service`), player reports
//! (`player_reports`), launcher reports (`reports`) and the server status with its maintenance banner (`status`).
//!
//! Everyone else gets 404 for every admin path. Staff act only on accounts and roles ranked
//! below their own (see `staff::outranks`); every change is written to the audit log.

use std::sync::Arc;

use askama::Template;
use axum::Router;
use axum::extract::{Form, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use serde::Deserialize;

use crate::db::{self, AssignOutcome, ReserveOutcome, Session};
use crate::names;
use crate::staff::{self, Role};
use crate::web::{
    AppError, AppState, PageResult, Shared, Stamp, csrf_failed, csrf_ok, message, page,
    require_session, stamp,
};

/// An early response from a guard, boxed because responses are large.
type Reject = Box<Response>;

const PAGE_SIZE: i64 = 50;
const MAX_REASON: usize = 200;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .merge(service::routes())
        .merge(reports::routes())
        .merge(player_reports::routes())
        .route("/admin", get(dashboard))
        .route("/admin/users", get(users))
        .route("/admin/users/{id}", get(user_page))
        .route("/admin/users/{id}/ban", post(user_ban))
        .route("/admin/users/{id}/unban", post(user_unban))
        .route(
            "/admin/users/{id}/devices/{device}/revoke",
            post(user_revoke_device),
        )
        .route("/admin/users/{id}/assign-name", post(user_assign_name))
        .route("/admin/users/{id}/delete", post(user_delete))
        .route("/admin/names", get(names_page))
        .route("/admin/names/reserve", post(names_reserve))
        .route("/admin/names/release", post(names_release))
        .route("/admin/staff", get(staff_page))
        .route("/admin/staff/set", post(staff_set))
        .route("/admin/staff/remove", post(staff_remove))
        .merge(game::routes())
        .merge(status::routes())
}

struct Staff {
    session: Session,
    role: Role,
}

impl Staff {
    fn account(&self) -> &db::Account {
        &self.session.account
    }
    fn nav(&self) -> Nav {
        Nav {
            nav_csrf: Some(self.session.csrf_token.clone()),
            csrf: self.session.csrf_token.clone(),
            role: self.role.as_str(),
            is_admin: self.role >= Role::Admin,
        }
    }
}

/// Fields every admin page's layout uses.
struct Nav {
    nav_csrf: Option<String>,
    csrf: String,
    role: &'static str,
    is_admin: bool,
}

fn not_found() -> Response {
    message(
        StatusCode::NOT_FOUND,
        "Not found",
        "There is no page here.",
        Some(("/", "Home")),
    )
}

fn forbidden(what: &str) -> Response {
    message(
        StatusCode::FORBIDDEN,
        "Not allowed",
        what,
        Some(("/admin", "Admin home")),
    )
}

fn internal(e: impl Into<anyhow::Error>) -> Response {
    AppError::from(e).into_response()
}

/// The signed-in staff member with at least `min`, or the response to send instead.
async fn require_staff(
    state: &AppState,
    headers: &HeaderMap,
    path: &str,
    min: Role,
) -> Result<Staff, Reject> {
    let session = require_session(state, headers, path)
        .await
        .map_err(|r| *r)?;
    let role = staff::role_for_email(
        &state.pool,
        &state.settings.owner_emails,
        session.account.email.as_deref(),
    )
    .await
    .map_err(|e| Box::new(internal(e)))?;
    match role {
        None => Err(Box::new(not_found())),
        Some(r) if r < min => Err(Box::new(forbidden("Your role cannot do that."))),
        Some(role) => Ok(Staff { session, role }),
    }
}

/// As `require_staff`, for a form that must carry the session's CSRF token.
async fn require_staff_form(
    state: &AppState,
    headers: &HeaderMap,
    min: Role,
    csrf: &str,
) -> Result<Staff, Reject> {
    let staff = require_staff(state, headers, "/admin", min).await?;
    if !csrf_ok(&staff.session, csrf) {
        return Err(Box::new(csrf_failed()));
    }
    Ok(staff)
}

/// The target account and its role, if `actor` outranks it.
async fn target_account(
    state: &AppState,
    actor: &Staff,
    id: i32,
) -> Result<(db::Account, Option<Role>), Reject> {
    let Some(account) = db::account(&state.pool, id)
        .await
        .map_err(|e| Box::new(internal(e)))?
    else {
        return Err(Box::new(message(
            StatusCode::NOT_FOUND,
            "No such account",
            "That account does not exist.",
            Some(("/admin/users", "Users")),
        )));
    };
    let role = staff::role_for_email(
        &state.pool,
        &state.settings.owner_emails,
        account.email.as_deref(),
    )
    .await
    .map_err(|e| Box::new(internal(e)))?;
    if !staff::outranks(actor.role, role) {
        return Err(Box::new(forbidden(
            "You can only act on accounts whose role is below yours.",
        )));
    }
    if !may_change(actor.role, &account, role) {
        return Err(Box::new(forbidden(
            "Only admins can change service accounts.",
        )));
    }
    Ok((account, role))
}

/// Whether `actor` may change `account` (holding `role`): only accounts ranked below them,
/// and service accounts (operator test identities, `service`) only from admin up, so
/// moderators can see them but not ban them or unlink their launchers.
fn may_change(actor: Role, account: &db::Account, role: Option<Role>) -> bool {
    staff::outranks(actor, role) && (actor >= Role::Admin || !account.is_service())
}

fn describe(account: &db::Account) -> String {
    format!(
        "account {} ({})",
        account.id,
        account.player_name.as_deref().unwrap_or("no name")
    )
}

macro_rules! guard {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(response) => return Ok(*response),
        }
    };
}

mod game;
mod status;

// ---- Dashboard ----

struct LogView {
    when: Stamp,
    actor: String,
    action: String,
    target: String,
    detail: String,
}

#[derive(Template)]
#[template(path = "admin_dashboard.html")]
struct DashboardPage {
    nav_csrf: Option<String>,
    role: &'static str,
    is_admin: bool,
    accounts: i64,
    named: i64,
    banned: i64,
    devices: i64,
    reserved: i64,
    open_player_reports: i64,
    log: Vec<LogView>,
}

async fn dashboard(State(state): Shared, headers: HeaderMap) -> PageResult {
    let actor = guard!(require_staff(&state, &headers, "/admin", Role::Moderator).await);
    let stats = staff::stats(&state.pool).await?;
    let log = staff::recent_log(&state.pool, 100)
        .await?
        .into_iter()
        .map(|l| LogView {
            when: stamp(l.created_at),
            actor: l.actor_email,
            action: l.action,
            target: l.target,
            detail: l.detail,
        })
        .collect();
    let Nav {
        nav_csrf,
        csrf: _,
        role,
        is_admin,
    } = actor.nav();
    Ok(page(
        StatusCode::OK,
        DashboardPage {
            nav_csrf,
            role,
            is_admin,
            accounts: stats.accounts,
            named: stats.named,
            banned: stats.banned,
            devices: stats.devices,
            reserved: stats.reserved,
            open_player_reports: crate::player_reports::open_count(&state.pool).await?,
            log,
        },
    ))
}

// ---- Users ----

#[derive(Deserialize)]
struct UsersQuery {
    q: Option<String>,
    page: Option<i64>,
}

struct UserRow {
    id: i32,
    name: String,
    email: String,
    created: Stamp,
    banned: bool,
    devices: i64,
    service: bool,
}

#[derive(Template)]
#[template(path = "admin_users.html")]
struct UsersPage {
    nav_csrf: Option<String>,
    role: &'static str,
    is_admin: bool,
    q: String,
    page: i64,
    has_next: bool,
    rows: Vec<UserRow>,
}

async fn users(
    State(state): Shared,
    headers: HeaderMap,
    Query(query): Query<UsersQuery>,
) -> PageResult {
    let actor = guard!(require_staff(&state, &headers, "/admin/users", Role::Moderator).await);
    let q: String = query
        .q
        .unwrap_or_default()
        .trim()
        .chars()
        .take(100)
        .collect();
    let page_no = query.page.unwrap_or(0).clamp(0, 10_000);
    let mut found =
        staff::search_accounts(&state.pool, &q, PAGE_SIZE + 1, page_no * PAGE_SIZE).await?;
    let has_next = found.len() as i64 > PAGE_SIZE;
    found.truncate(PAGE_SIZE as usize);
    let rows = found
        .into_iter()
        .map(|a| UserRow {
            id: a.id,
            name: a.player_name.unwrap_or_default(),
            email: a.email.unwrap_or_default(),
            created: stamp(a.created_at),
            banned: a.banned_at.is_some(),
            devices: a.devices,
            service: a.kind == db::SERVICE_KIND,
        })
        .collect();
    let Nav {
        nav_csrf,
        csrf: _,
        role,
        is_admin,
    } = actor.nav();
    Ok(page(
        StatusCode::OK,
        UsersPage {
            nav_csrf,
            role,
            is_admin,
            q,
            page: page_no,
            has_next,
            rows,
        },
    ))
}

/// An account's ban as the admin pages show it.
struct BanView {
    since: Stamp,
    reason: String,
}

struct DeviceView {
    id: i32,
    label: String,
    created: Stamp,
    last_used: Stamp,
}

#[derive(Template)]
#[template(path = "admin_user.html")]
struct UserPage {
    nav_csrf: Option<String>,
    csrf: String,
    role: &'static str,
    is_admin: bool,
    id: i32,
    name: Option<String>,
    email: String,
    provider: String,
    created: Stamp,
    ban: Option<BanView>,
    target_role: Option<&'static str>,
    can_act: bool,
    devices: Vec<DeviceView>,
    game_href: Option<String>,
    social: crate::social::Counts,
}

async fn user_page(State(state): Shared, headers: HeaderMap, Path(id): Path<i32>) -> PageResult {
    let path = format!("/admin/users/{id}");
    let actor = guard!(require_staff(&state, &headers, &path, Role::Moderator).await);
    let Some(account) = db::account(&state.pool, id).await? else {
        return Ok(not_found());
    };
    let target_role = staff::role_for_email(
        &state.pool,
        &state.settings.owner_emails,
        account.email.as_deref(),
    )
    .await?;
    let devices = db::devices(&state.pool, id)
        .await?
        .into_iter()
        .map(|d| DeviceView {
            id: d.id,
            label: d.label,
            created: stamp(d.created_at),
            last_used: d.last_used_at.map_or_else(|| Stamp::text("never"), stamp),
        })
        .collect();
    let social = crate::social::counts(&state.pool, id).await?;
    let ban = account.banned_at.map(|at| BanView {
        since: stamp(at),
        reason: account.ban_reason.clone().unwrap_or_default(),
    });
    let Nav {
        nav_csrf,
        csrf,
        role,
        is_admin,
    } = actor.nav();
    Ok(page(
        StatusCode::OK,
        UserPage {
            nav_csrf,
            csrf,
            role,
            is_admin,
            id,
            name: account.player_name.clone(),
            email: account.email.clone().unwrap_or_default(),
            provider: account.provider.clone(),
            created: stamp(account.created_at),
            ban,
            target_role: target_role.map(Role::as_str),
            can_act: may_change(actor.role, &account, target_role),
            devices,
            game_href: game::character_href(
                state.settings.game_admin.as_ref(),
                account.player_name.as_deref(),
            ),
            social,
        },
    ))
}

#[derive(Deserialize)]
struct CsrfOnly {
    csrf: String,
}

#[derive(Deserialize)]
struct BanForm {
    csrf: String,
    reason: String,
}

async fn user_ban(
    State(state): Shared,
    headers: HeaderMap,
    Path(id): Path<i32>,
    Form(form): Form<BanForm>,
) -> PageResult {
    let actor = guard!(require_staff_form(&state, &headers, Role::Moderator, &form.csrf).await);
    let (account, _) = guard!(target_account(&state, &actor, id).await);
    let reason = form.reason.trim();
    if reason.is_empty() || reason.chars().count() > MAX_REASON {
        return Ok(message(
            StatusCode::BAD_REQUEST,
            "Reason needed",
            "Give a ban reason of at most 200 characters; the player sees it.",
            Some((&format!("/admin/users/{id}"), "Back")),
        ));
    }
    staff::set_ban_by_id(&state.pool, id, Some(reason)).await?;
    staff::log(
        &state.pool,
        actor.account(),
        "ban",
        &describe(&account),
        reason,
    )
    .await?;
    Ok(Redirect::to(&format!("/admin/users/{id}")).into_response())
}

async fn user_unban(
    State(state): Shared,
    headers: HeaderMap,
    Path(id): Path<i32>,
    Form(form): Form<CsrfOnly>,
) -> PageResult {
    let actor = guard!(require_staff_form(&state, &headers, Role::Moderator, &form.csrf).await);
    let (account, _) = guard!(target_account(&state, &actor, id).await);
    staff::set_ban_by_id(&state.pool, id, None).await?;
    staff::log(
        &state.pool,
        actor.account(),
        "unban",
        &describe(&account),
        "",
    )
    .await?;
    Ok(Redirect::to(&format!("/admin/users/{id}")).into_response())
}

async fn user_revoke_device(
    State(state): Shared,
    headers: HeaderMap,
    Path((id, device)): Path<(i32, i32)>,
    Form(form): Form<CsrfOnly>,
) -> PageResult {
    let actor = guard!(require_staff_form(&state, &headers, Role::Moderator, &form.csrf).await);
    let (account, _) = guard!(target_account(&state, &actor, id).await);
    if db::revoke_device(&state.pool, id, device).await? {
        staff::log(
            &state.pool,
            actor.account(),
            "unlink launcher",
            &describe(&account),
            &format!("device {device}"),
        )
        .await?;
    }
    Ok(Redirect::to(&format!("/admin/users/{id}")).into_response())
}

#[derive(Deserialize)]
struct NameForm {
    csrf: String,
    name: String,
}

async fn user_assign_name(
    State(state): Shared,
    headers: HeaderMap,
    Path(id): Path<i32>,
    Form(form): Form<NameForm>,
) -> PageResult {
    let actor = guard!(require_staff_form(&state, &headers, Role::Admin, &form.csrf).await);
    let (account, _) = guard!(target_account(&state, &actor, id).await);
    let name = form.name.trim();
    let back = format!("/admin/users/{id}");
    let problem = match db::assign_reserved_name(&state.pool, name, id).await? {
        AssignOutcome::Assigned => {
            staff::log(
                &state.pool,
                actor.account(),
                "assign name",
                &describe(&account),
                name,
            )
            .await?;
            return Ok(Redirect::to(&back).into_response());
        }
        AssignOutcome::NotReserved => {
            "That name is not reserved. Reserve it first, or let the player claim it."
        }
        AssignOutcome::AccountNamed => "This account already has a name.",
        AssignOutcome::NoSuchAccount => "That account no longer exists.",
    };
    Ok(message(
        StatusCode::OK,
        "Name not assigned",
        problem,
        Some((&back, "Back")),
    ))
}

#[derive(Deserialize)]
struct DeleteForm {
    csrf: String,
    confirm: String,
}

async fn user_delete(
    State(state): Shared,
    headers: HeaderMap,
    Path(id): Path<i32>,
    Form(form): Form<DeleteForm>,
) -> PageResult {
    let actor = guard!(require_staff_form(&state, &headers, Role::Admin, &form.csrf).await);
    let (account, _) = guard!(target_account(&state, &actor, id).await);
    if form.confirm.trim() != id.to_string() {
        return Ok(message(
            StatusCode::BAD_REQUEST,
            "Account kept",
            "Type the account number to confirm. Nothing was deleted.",
            Some((&format!("/admin/users/{id}"), "Back")),
        ));
    }
    if let Some(reports) = &state.reports {
        crate::reports::delete_account_logs(&state.pool, reports.store.as_ref(), id).await;
    }
    db::delete_account(&state.pool, id).await?;
    staff::log(
        &state.pool,
        actor.account(),
        "delete account",
        &describe(&account),
        "",
    )
    .await?;
    Ok(Redirect::to("/admin/users").into_response())
}

// ---- Reserved names ----

struct ReservedView {
    name: String,
    reason: String,
    created: Stamp,
}

#[derive(Template)]
#[template(path = "admin_names.html")]
struct NamesPage {
    nav_csrf: Option<String>,
    csrf: String,
    role: &'static str,
    is_admin: bool,
    names: Vec<ReservedView>,
    results: Vec<String>,
}

async fn render_names(state: &AppState, actor: &Staff, results: Vec<String>) -> PageResult {
    let names = db::reserved_names(&state.pool)
        .await?
        .into_iter()
        .map(|r| ReservedView {
            name: r.name,
            reason: r.reason,
            created: stamp(r.created_at),
        })
        .collect();
    let Nav {
        nav_csrf,
        csrf,
        role,
        is_admin,
    } = actor.nav();
    Ok(page(
        StatusCode::OK,
        NamesPage {
            nav_csrf,
            csrf,
            role,
            is_admin,
            names,
            results,
        },
    ))
}

async fn names_page(State(state): Shared, headers: HeaderMap) -> PageResult {
    let actor = guard!(require_staff(&state, &headers, "/admin/names", Role::Moderator).await);
    render_names(&state, &actor, Vec::new()).await
}

#[derive(Deserialize)]
struct ReserveForm {
    csrf: String,
    names: String,
    reason: String,
}

async fn names_reserve(
    State(state): Shared,
    headers: HeaderMap,
    Form(form): Form<ReserveForm>,
) -> PageResult {
    let actor = guard!(require_staff_form(&state, &headers, Role::Admin, &form.csrf).await);
    let reason = match form.reason.trim() {
        "" => "existing player",
        r => r,
    };
    let reason: String = reason.chars().take(MAX_REASON).collect();
    let mut results = Vec::new();
    for name in form
        .names
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|n| !n.is_empty())
        .take(500)
    {
        let outcome = db::reserve_name(&state.pool, name, &reason).await?;
        let shown: String = name.chars().take(32).collect();
        results.push(match outcome {
            ReserveOutcome::Reserved => {
                staff::log(&state.pool, actor.account(), "reserve name", name, &reason).await?;
                format!("{shown}: reserved")
            }
            ReserveOutcome::AlreadyReserved => format!("{shown}: already reserved"),
            ReserveOutcome::InUse => format!("{shown}: an account already has it"),
            ReserveOutcome::Invalid => format!("{shown}: not a valid player name"),
        });
    }
    render_names(&state, &actor, results).await
}

async fn names_release(
    State(state): Shared,
    headers: HeaderMap,
    Form(form): Form<NameForm>,
) -> PageResult {
    let actor = guard!(require_staff_form(&state, &headers, Role::Admin, &form.csrf).await);
    let name = form.name.trim();
    if names::valid_player_name(name) && db::release_name(&state.pool, name).await? {
        staff::log(&state.pool, actor.account(), "release name", name, "").await?;
    }
    Ok(Redirect::to("/admin/names").into_response())
}

// ---- Staff ----

struct StaffView {
    email: String,
    role: String,
    granted_by: String,
    created: Stamp,
    editable: bool,
}

#[derive(Template)]
#[template(path = "admin_staff.html")]
struct StaffPage {
    nav_csrf: Option<String>,
    csrf: String,
    role: &'static str,
    is_admin: bool,
    owners: Vec<String>,
    staff: Vec<StaffView>,
    grantable: Vec<&'static str>,
    error: Option<String>,
}

async fn render_staff(state: &AppState, actor: &Staff, error: Option<String>) -> PageResult {
    let staff = staff::staff(&state.pool)
        .await?
        .into_iter()
        .map(|s| {
            let held = Role::grantable(&s.role);
            StaffView {
                editable: staff::outranks(actor.role, held),
                email: s.email_key,
                role: s.role,
                granted_by: s.granted_by_name.unwrap_or_default(),
                created: stamp(s.created_at),
            }
        })
        .collect();
    let grantable = [Role::Moderator, Role::Admin]
        .into_iter()
        .filter(|r| actor.role > *r)
        .map(Role::as_str)
        .collect();
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
        StaffPage {
            nav_csrf,
            csrf,
            role,
            is_admin,
            owners: state.settings.owner_emails.clone(),
            staff,
            grantable,
            error,
        },
    ))
}

async fn staff_page(State(state): Shared, headers: HeaderMap) -> PageResult {
    let actor = guard!(require_staff(&state, &headers, "/admin/staff", Role::Admin).await);
    render_staff(&state, &actor, None).await
}

#[derive(Deserialize)]
struct StaffForm {
    csrf: String,
    email: String,
    role: String,
}

async fn staff_set(
    State(state): Shared,
    headers: HeaderMap,
    Form(form): Form<StaffForm>,
) -> PageResult {
    let actor = guard!(require_staff_form(&state, &headers, Role::Admin, &form.csrf).await);
    let email = staff::email_key(&form.email);
    let Some(new_role) = Role::grantable(&form.role) else {
        return render_staff(&state, &actor, Some("Choose admin or moderator.".into())).await;
    };
    if !staff::plausible_email(&email) {
        return render_staff(
            &state,
            &actor,
            Some("Enter the Google email the person signs in with.".into()),
        )
        .await;
    }
    let current =
        staff::role_for_email(&state.pool, &state.settings.owner_emails, Some(&email)).await?;
    if actor.role <= new_role || !staff::outranks(actor.role, current) {
        return Ok(forbidden(
            "You can only grant and change roles below your own.",
        ));
    }
    staff::set_staff(&state.pool, &email, new_role, actor.account().id).await?;
    staff::log(
        &state.pool,
        actor.account(),
        "grant role",
        &email,
        new_role.as_str(),
    )
    .await?;
    Ok(Redirect::to("/admin/staff").into_response())
}

#[derive(Deserialize)]
struct StaffRemoveForm {
    csrf: String,
    email: String,
}

async fn staff_remove(
    State(state): Shared,
    headers: HeaderMap,
    Form(form): Form<StaffRemoveForm>,
) -> PageResult {
    let actor = guard!(require_staff_form(&state, &headers, Role::Admin, &form.csrf).await);
    let email = staff::email_key(&form.email);
    let current = staff::stored_role(&state.pool, &email).await?;
    if current.is_none() {
        return Ok(Redirect::to("/admin/staff").into_response());
    }
    if !staff::outranks(actor.role, current) {
        return Ok(forbidden("You can only remove roles below your own."));
    }
    staff::remove_staff(&state.pool, &email).await?;
    staff::log(&state.pool, actor.account(), "remove role", &email, "").await?;
    Ok(Redirect::to("/admin/staff").into_response())
}

// Service accounts, player reports and launcher reports; after `guard!`, which they use.
mod player_reports;
mod reports;
mod service;
