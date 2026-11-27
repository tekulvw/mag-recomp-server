//! The admin panel's Game server section under `/admin/game`: characters on the game server
//! and level changes, through the game server's internal admin API (`crate::game_admin`).
//!
//! Moderators and above can view, and can mute or ban an account's in-match voice (or lift
//! it). Admins and above can set levels. Every change is a dry run first that shows the plan,
//! and a separate confirmation applies it. Applied changes go to the audit log.

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
use crate::game_admin::{
    GameAdmin, GameAdminError, MAX_VOICE_MINUTES, Refusal, VoiceAction, VoiceSanction,
    valid_account,
};
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
        .route("/admin/game/characters/{account}/voice", post(voice_plan))
        .route(
            "/admin/game/characters/{account}/voice/confirm",
            post(voice_apply),
        )
}

/// One voice sanction as the pages show it.
struct SanctionRow {
    account: String,
    kind: String,
    ends: String,
    since: String,
    operator: String,
    reason: String,
}

impl From<VoiceSanction> for SanctionRow {
    fn from(s: VoiceSanction) -> Self {
        Self {
            account: s.account,
            kind: s.kind,
            ends: s.expires_at.unwrap_or_else(|| "until lifted".into()),
            since: s.created_at,
            operator: s.operator,
            reason: s.reason.unwrap_or_default(),
        }
    }
}

/// The active sanctions, or why they cannot be shown (an older game server answers 404).
async fn sanctions(api: &GameAdmin, actor: &str) -> (Vec<SanctionRow>, Option<String>) {
    match api.voice_sanctions(actor).await {
        Ok(list) => (list.into_iter().map(SanctionRow::from).collect(), None),
        Err(GameAdminError::NotFound) => (
            Vec::new(),
            Some("This game server does not support voice moderation yet.".into()),
        ),
        Err(e) => (Vec::new(), Some(e.to_string())),
    }
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
        GameAdminError::Uncertain { .. } => StatusCode::GATEWAY_TIMEOUT,
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
    sanctions: Vec<SanctionRow>,
    voice_error: Option<String>,
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
        sanctions: Vec::new(),
        voice_error: None,
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
    (view.sanctions, view.voice_error) = sanctions(api, &actor_label(&actor)).await;
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
    sanctions: Vec<SanctionRow>,
    voice_error: Option<String>,
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
        sanctions: Vec::new(),
        voice_error: None,
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
            let (all, voice_error) = sanctions(api, &actor_label(&actor)).await;
            view.sanctions = all
                .into_iter()
                .filter(|s| s.account == view.account)
                .collect();
            view.voice_error = voice_error;
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
    /// An applied change the game server did not confirm; it may have been made.
    uncertain: Option<String>,
    /// The level read back after an unconfirmed change, or why it could not be read.
    now_level: Option<u32>,
    now_error: Option<String>,
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
        uncertain: None,
        now_level: None,
        now_error: None,
    };
    let change = match result {
        Ok(change) => change,
        Err(GameAdminError::NotFound) => return Ok(no_character(&view.account)),
        Err(GameAdminError::Refused(r)) => {
            view.refusal = Some(r.into());
            return Ok(page(StatusCode::CONFLICT, view));
        }
        Err(e @ GameAdminError::Uncertain { .. }) => {
            // Read back what the game server holds now, and keep the attempt in the audit log.
            match api.character(&actor_label(&actor), &view.account).await {
                Ok(Some(c)) => view.now_level = Some(c.level),
                Ok(None) => view.now_error = Some("the game server has no character for it".into()),
                Err(read) => view.now_error = Some(read.to_string()),
            }
            staff::log(
                &state.pool,
                actor.account(),
                "set level",
                &view.account,
                &format!("to {level}, unconfirmed: the game server did not confirm it"),
            )
            .await?;
            view.uncertain = Some(e.to_string());
            return Ok(page(upstream_status(&e), view));
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

// ---- Voice mutes and bans ----

#[derive(Template)]
#[template(path = "admin_game_voice.html")]
struct VoicePage {
    nav_csrf: Option<String>,
    csrf: String,
    role: &'static str,
    is_admin: bool,
    account: String,
    action: &'static str,
    lift: bool,
    /// The form's duration in minutes, empty for "until lifted"; echoed into the confirmation.
    duration: String,
    reason: String,
    applied: bool,
    before: Option<SanctionRow>,
    after: Option<SanctionRow>,
    refusal: Option<RefusalView>,
    error: Option<String>,
    /// An applied change the game server did not confirm; it may have been made.
    uncertain: Option<String>,
    /// The account's sanctions read back after an unconfirmed change, or why they could not be read.
    now: Vec<SanctionRow>,
    now_error: Option<String>,
}

#[derive(Deserialize)]
struct VoiceForm {
    csrf: String,
    action: String,
    #[serde(default)]
    duration: String,
    #[serde(default)]
    reason: String,
}

async fn voice_plan(
    State(state): Shared,
    headers: HeaderMap,
    Path(raw): Path<String>,
    Form(form): Form<VoiceForm>,
) -> PageResult {
    change_voice(&state, &headers, &raw, &form, false).await
}

async fn voice_apply(
    State(state): Shared,
    headers: HeaderMap,
    Path(raw): Path<String>,
    Form(form): Form<VoiceForm>,
) -> PageResult {
    change_voice(&state, &headers, &raw, &form, true).await
}

async fn change_voice(
    state: &AppState,
    headers: &HeaderMap,
    raw: &str,
    form: &VoiceForm,
    apply: bool,
) -> PageResult {
    let actor = guard!(require_staff_form(state, headers, Role::Moderator, &form.csrf).await);
    let Some(account) = account_key(raw) else {
        return Ok(no_character(&raw.chars().take(32).collect::<String>()));
    };
    let back = format!("/admin/game/characters/{account}");
    let Some(api) = state.settings.game_admin.as_ref() else {
        return Ok(not_configured_message());
    };
    let bad = |title: &str, text: &str| {
        Ok(message(
            StatusCode::BAD_REQUEST,
            title,
            &format!("{text} Nothing was changed."),
            Some((&back, "Back")),
        ))
    };
    let Some(action) = VoiceAction::parse(form.action.trim()) else {
        return bad("Choose an action", "Choose mute, ban, unmute or unban.");
    };
    let duration_text = form.duration.trim();
    let duration = if duration_text.is_empty() || action.is_lift() {
        None
    } else {
        match duration_text.parse::<u32>() {
            Ok(m) if (1..=MAX_VOICE_MINUTES).contains(&m) => Some(m),
            _ => {
                return bad(
                    "Duration needed",
                    "Choose how long the sanction lasts, or until lifted.",
                );
            }
        }
    };
    let reason = form.reason.trim();
    if reason.len() > 256 || !reason.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
        return bad(
            "Reason too long",
            "Keep the reason to 256 plain characters (letters, digits and punctuation).",
        );
    }
    let result = api
        .change_voice(
            &actor_label(&actor),
            &account,
            action,
            apply,
            (!reason.is_empty()).then_some(reason),
            duration,
        )
        .await;
    let Nav {
        nav_csrf,
        csrf,
        role,
        is_admin,
    } = actor.nav();
    let mut view = VoicePage {
        nav_csrf,
        csrf,
        role,
        is_admin,
        account,
        action: action.as_str(),
        lift: action.is_lift(),
        duration: duration.map(|m| m.to_string()).unwrap_or_default(),
        reason: reason.to_owned(),
        applied: false,
        before: None,
        after: None,
        refusal: None,
        error: None,
        uncertain: None,
        now: Vec::new(),
        now_error: None,
    };
    let change = match result {
        Ok(change) => change,
        Err(GameAdminError::NotFound) => return Ok(no_character(&view.account)),
        Err(GameAdminError::Refused(r)) => {
            view.refusal = Some(r.into());
            return Ok(page(StatusCode::CONFLICT, view));
        }
        Err(e @ GameAdminError::Uncertain { .. }) => {
            // Read back what the game server holds now, and keep the attempt in the audit log.
            let (all, read_error) = sanctions(api, &actor_label(&actor)).await;
            view.now = all
                .into_iter()
                .filter(|s| s.account == view.account)
                .collect();
            view.now_error = read_error;
            staff::log(
                &state.pool,
                actor.account(),
                &format!("voice {}", action.as_str()),
                &view.account,
                "unconfirmed: the game server did not confirm it",
            )
            .await?;
            view.uncertain = Some(e.to_string());
            return Ok(page(upstream_status(&e), view));
        }
        Err(e) => {
            let status = upstream_status(&e);
            view.error = Some(e.to_string());
            return Ok(page(status, view));
        }
    };
    if change.applied {
        let ends = change
            .after
            .as_ref()
            .map(|s| s.expires_at.as_deref().unwrap_or("until lifted").to_owned());
        let detail = match ends {
            Some(ends) => format!("ends {ends}"),
            None => format!("lifted {}", change.kind),
        };
        let detail = if reason.is_empty() {
            detail
        } else {
            format!("{detail}; reason: {reason}")
        };
        staff::log(
            &state.pool,
            actor.account(),
            &format!("voice {}", action.as_str()),
            &view.account,
            &detail,
        )
        .await?;
    }
    view.applied = change.applied;
    view.before = change.before.map(SanctionRow::from);
    view.after = change.after.map(SanctionRow::from);
    Ok(page(StatusCode::OK, view))
}

/// For the admin user page: the game character path for a player name, when the section is on.
pub(super) fn character_href(api: Option<&GameAdmin>, player_name: Option<&str>) -> Option<String> {
    api?;
    account_key(player_name?).map(|k| format!("/admin/game/characters/{k}"))
}
