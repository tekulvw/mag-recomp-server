//! The admin panel's Game server section under `/admin/game`: characters on the game server
//! and level changes, through the game server's internal admin API (`crate::game_admin`).
//!
//! Moderators and above can view. Admins and above can set levels: a dry run first shows
//! the plan, and a separate confirmation applies it. Applied changes go to the audit log.

use std::collections::HashMap;
use std::sync::Arc;

use askama::Template;
use axum::Router;
use axum::extract::{Form, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use serde::Deserialize;

use super::{Nav, Staff, require_staff, require_staff_form};
use crate::db;
use crate::game_admin::{GameAdmin, GameAdminError, Refusal, valid_account};
use crate::staff::{self, Role};
use crate::web::{AppState, PageResult, Shared, message, page};

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/admin/game", get(list))
        .route("/admin/game/characters/{account}", get(character_page))
        .route("/admin/game/characters/{account}/level", post(level_plan))
        .route(
            "/admin/game/characters/{account}/level/confirm",
            post(level_apply),
        )
}

/// Who the game server records as acting: the staff member's email, else their account.
fn actor_label(actor: &Staff) -> String {
    let account = actor.account();
    match account.email.as_deref() {
        Some(e) if !e.is_empty() && e.len() <= 254 && e.bytes().all(|b| b.is_ascii_graphic()) => {
            e.to_owned()
        }
        _ => format!("account {}", account.id),
    }
}

/// The game server's account key for a name in a URL: the lowercase name.
fn account_key(raw: &str) -> Option<String> {
    let key = raw.to_ascii_lowercase();
    valid_account(&key).then_some(key)
}

fn upstream_status(e: &GameAdminError) -> StatusCode {
    match e {
        GameAdminError::NotFound => StatusCode::NOT_FOUND,
        GameAdminError::Refused(_) => StatusCode::CONFLICT,
        GameAdminError::BadRequest(_) => StatusCode::BAD_REQUEST,
        _ => StatusCode::BAD_GATEWAY,
    }
}

fn no_character(account: &str) -> axum::response::Response {
    message(
        StatusCode::NOT_FOUND,
        "No such character",
        &format!("The game server has no character for {account}."),
        Some(("/admin/game", "Game server")),
    )
}

fn not_configured_message() -> axum::response::Response {
    message(
        StatusCode::SERVICE_UNAVAILABLE,
        "Game server not configured",
        "The portal has no game server admin API configured, so nothing was changed.",
        Some(("/admin/game", "Game server")),
    )
}

// ---- List ----

struct CharacterRow {
    account: String,
    level: u32,
    experience: i64,
    skill_points: i64,
    session: &'static str,
    portal_id: Option<i32>,
}

#[derive(Template)]
#[template(path = "admin_game.html")]
struct ListPage {
    nav_csrf: Option<String>,
    role: &'static str,
    is_admin: bool,
    configured: bool,
    error: Option<String>,
    rows: Vec<CharacterRow>,
}

async fn list(State(state): Shared, headers: HeaderMap) -> PageResult {
    let actor = guard!(require_staff(&state, &headers, "/admin/game", Role::Moderator).await);
    let Nav {
        nav_csrf,
        csrf: _,
        role,
        is_admin,
    } = actor.nav();
    let mut view = ListPage {
        nav_csrf,
        role,
        is_admin,
        configured: false,
        error: None,
        rows: Vec::new(),
    };
    let Some(api) = state.settings.game_admin.as_ref() else {
        return Ok(page(StatusCode::OK, view));
    };
    view.configured = true;
    let mut characters = match api.accounts(&actor_label(&actor)).await {
        Ok(c) => c,
        Err(e) => {
            let status = upstream_status(&e);
            view.error = Some(e.to_string());
            return Ok(page(status, view));
        }
    };
    characters.sort_by(|a, b| a.account.cmp(&b.account));
    let keys: Vec<String> = characters
        .iter()
        .map(|c| c.account.to_ascii_lowercase())
        .collect();
    let ids: HashMap<String, i32> = db::account_ids_by_name_keys(&state.pool, &keys)
        .await?
        .into_iter()
        .collect();
    view.rows = characters
        .into_iter()
        .map(|c| CharacterRow {
            portal_id: ids.get(&c.account.to_ascii_lowercase()).copied(),
            account: c.account,
            level: c.level,
            experience: c.experience,
            skill_points: c.skill_points,
            session: c.session.as_str(),
        })
        .collect();
    Ok(page(StatusCode::OK, view))
}

// ---- One character ----

struct CharacterView {
    level: u32,
    experience: i64,
    skill_points: i64,
    next_level: String,
    session: &'static str,
}

#[derive(Template)]
#[template(path = "admin_game_character.html")]
struct CharacterPage {
    nav_csrf: Option<String>,
    csrf: String,
    role: &'static str,
    is_admin: bool,
    configured: bool,
    account: String,
    portal_id: Option<i32>,
    character: Option<CharacterView>,
    error: Option<String>,
}

async fn character_page(
    State(state): Shared,
    headers: HeaderMap,
    Path(raw): Path<String>,
) -> PageResult {
    let path = format!("/admin/game/characters/{raw}");
    let actor = guard!(require_staff(&state, &headers, &path, Role::Moderator).await);
    let Some(account) = account_key(&raw) else {
        return Ok(no_character(&raw.chars().take(32).collect::<String>()));
    };
    let Nav {
        nav_csrf,
        csrf,
        role,
        is_admin,
    } = actor.nav();
    let portal_id = db::account_by_name(&state.pool, &account)
        .await?
        .map(|a| a.id);
    let mut view = CharacterPage {
        nav_csrf,
        csrf,
        role,
        is_admin,
        configured: false,
        account,
        portal_id,
        character: None,
        error: None,
    };
    let Some(api) = state.settings.game_admin.as_ref() else {
        return Ok(page(StatusCode::OK, view));
    };
    view.configured = true;
    match api.character(&actor_label(&actor), &view.account).await {
        Ok(Some(c)) => {
            view.character = Some(CharacterView {
                level: c.level,
                experience: c.experience,
                skill_points: c.skill_points,
                next_level: c
                    .next_level_experience
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "top level".into()),
                session: c.session.as_str(),
            });
            Ok(page(StatusCode::OK, view))
        }
        Ok(None) => Ok(no_character(&view.account)),
        Err(e) => {
            let status = upstream_status(&e);
            view.error = Some(e.to_string());
            Ok(page(status, view))
        }
    }
}

// ---- Set level ----

struct ChangeView {
    from_level: u32,
    to_level: u32,
    experience_before: i64,
    experience_after: i64,
    skill_points_before: i64,
    skill_points_after: i64,
    backup: Option<String>,
}

struct RefusalView {
    code: String,
    message: String,
    advice: &'static str,
}

impl From<Refusal> for RefusalView {
    fn from(r: Refusal) -> Self {
        Self {
            advice: r.advice(),
            code: r.code,
            message: r.message,
        }
    }
}

#[derive(Template)]
#[template(path = "admin_game_level.html")]
struct LevelPage {
    nav_csrf: Option<String>,
    csrf: String,
    role: &'static str,
    is_admin: bool,
    account: String,
    level: u32,
    applied: bool,
    change: Option<ChangeView>,
    refusal: Option<RefusalView>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct LevelForm {
    csrf: String,
    level: String,
}

async fn level_plan(
    State(state): Shared,
    headers: HeaderMap,
    Path(raw): Path<String>,
    Form(form): Form<LevelForm>,
) -> PageResult {
    set_level(&state, &headers, &raw, &form, false).await
}

async fn level_apply(
    State(state): Shared,
    headers: HeaderMap,
    Path(raw): Path<String>,
    Form(form): Form<LevelForm>,
) -> PageResult {
    set_level(&state, &headers, &raw, &form, true).await
}

async fn set_level(
    state: &AppState,
    headers: &HeaderMap,
    raw: &str,
    form: &LevelForm,
    apply: bool,
) -> PageResult {
    let actor = guard!(require_staff_form(state, headers, Role::Admin, &form.csrf).await);
    let Some(account) = account_key(raw) else {
        return Ok(no_character(&raw.chars().take(32).collect::<String>()));
    };
    let back = format!("/admin/game/characters/{account}");
    let Some(api) = state.settings.game_admin.as_ref() else {
        return Ok(not_configured_message());
    };
    let Some(level) = form.level.trim().parse::<u32>().ok().filter(|l| *l >= 1) else {
        return Ok(message(
            StatusCode::BAD_REQUEST,
            "Level needed",
            "Enter the new level as a whole number. Nothing was changed.",
            Some((&back, "Back")),
        ));
    };
    let result = api
        .set_level(&actor_label(&actor), &account, level, apply)
        .await;
    let Nav {
        nav_csrf,
        csrf,
        role,
        is_admin,
    } = actor.nav();
    let mut view = LevelPage {
        nav_csrf,
        csrf,
        role,
        is_admin,
        account,
        level,
        applied: false,
        change: None,
        refusal: None,
        error: None,
    };
    let change = match result {
        Ok(change) => change,
        Err(GameAdminError::NotFound) => return Ok(no_character(&view.account)),
        Err(GameAdminError::Refused(r)) => {
            view.refusal = Some(r.into());
            return Ok(page(StatusCode::CONFLICT, view));
        }
        Err(e) => {
            let status = upstream_status(&e);
            view.error = Some(e.to_string());
            return Ok(page(status, view));
        }
    };
    if change.applied {
        staff::log(
            &state.pool,
            actor.account(),
            "set level",
            &view.account,
            &format!(
                "{}→{}, backup {}",
                change.from_level,
                change.to_level,
                change.backup.as_deref().unwrap_or("none")
            ),
        )
        .await?;
    }
    view.applied = change.applied;
    view.change = Some(ChangeView {
        from_level: change.from_level,
        to_level: change.to_level,
        experience_before: change.experience_before,
        experience_after: change.experience_after,
        skill_points_before: change.skill_points_before,
        skill_points_after: change.skill_points_after,
        backup: change.backup,
    });
    Ok(page(StatusCode::OK, view))
}

/// For the admin user page: the game character path for a player name, when the section is on.
pub(super) fn character_href(api: Option<&GameAdmin>, player_name: Option<&str>) -> Option<String> {
    api?;
    account_key(player_name?).map(|k| format!("/admin/game/characters/{k}"))
}
