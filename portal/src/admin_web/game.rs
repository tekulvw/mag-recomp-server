//! The admin panel's Game server section under `/admin/game`: characters on the game server
//! and level changes, through the game server's internal admin API (`crate::game_admin`).
//!
//! An account holds up to three characters, in slots 0 to 2; the list shows each one (deleted
//! ones marked), and a character page shows all of an account's characters and works on the one
//! in its `?slot=`. A game server that predates slots has one character per account and none of
//! the slot fields; the pages then look as they did before.
//!
//! Moderators and above can view, and can mute or ban an account's in-match voice (or lift
//! it). Admins and above can set levels. Every change is a dry run first that shows the plan,
//! and a separate confirmation applies it. Applied changes go to the audit log.

use std::collections::HashMap;
use std::sync::Arc;

use askama::Template;
use axum::Router;
use axum::extract::{Form, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use serde::Deserialize;

use super::{Nav, Staff, require_staff, require_staff_form};
use crate::db;
use crate::game_admin::{
    AccountCharacter, CharacterSummary, GameAdmin, GameAdminError, MAX_SLOT, MAX_VOICE_MINUTES,
    Refusal, VoiceAction, VoiceSanction, valid_account,
};
use crate::staff::{self, Role};
use crate::status::faction_name;
use crate::web::{AppState, PageResult, Shared, Stamp, message, page};

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
    ends: Stamp,
    since: Stamp,
    operator: String,
    reason: String,
}

impl From<VoiceSanction> for SanctionRow {
    fn from(s: VoiceSanction) -> Self {
        Self {
            account: s.account,
            kind: s.kind,
            ends: s
                .expires_at
                .as_deref()
                .map_or_else(|| Stamp::text("until lifted"), Stamp::parse),
            since: Stamp::parse(&s.created_at),
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

/// A slot from a query or form: empty is none, anything else must be a slot number.
fn parse_slot(raw: Option<&str>) -> Result<Option<u8>, ()> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(text) => match text.parse::<u8>() {
            Ok(slot) if slot <= MAX_SLOT => Ok(Some(slot)),
            _ => Err(()),
        },
    }
}

/// `?slot=N` for a character page link or form action, empty without a slot.
fn slot_query(slot: Option<u8>) -> String {
    slot.map(|s| format!("?slot={s}")).unwrap_or_default()
}

fn no_character(account: &str) -> axum::response::Response {
    message(
        StatusCode::NOT_FOUND,
        "No such character",
        &format!("The game server has no character for {account}."),
        Some(("/admin/game", "Game server")),
    )
}

fn no_character_in(account: &str, slot: Option<u8>) -> axum::response::Response {
    match slot {
        None => no_character(account),
        Some(slot) => message(
            StatusCode::NOT_FOUND,
            "No such character",
            &format!("The game server has no live character in slot {slot} of {account}."),
            Some((&format!("/admin/game/characters/{account}"), "Back")),
        ),
    }
}

fn bad_slot(back: &str) -> axum::response::Response {
    message(
        StatusCode::BAD_REQUEST,
        "No such slot",
        &format!("Character slots are numbered 0 to {MAX_SLOT}. Nothing was changed."),
        Some((back, "Back")),
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
    /// The slot the row's link opens: `None` (the account's page) for a deleted character,
    /// whose slot is free, and from a game server without slots.
    link_slot: Option<u8>,
    /// `None` from a game server that predates character slots.
    slot: Option<u8>,
    name: String,
    faction: &'static str,
    level: u32,
    experience: i64,
    skill_points: i64,
    session: &'static str,
    /// When the player deleted it: `Some` for a deleted character ("" if the time is unknown).
    deleted_at: Option<Stamp>,
    portal_id: Option<i32>,
}

impl CharacterRow {
    fn new(c: CharacterSummary, portal_id: Option<i32>) -> Self {
        Self {
            link_slot: c.slot.filter(|_| !c.deleted),
            slot: c.slot,
            name: c.name.unwrap_or_default(),
            faction: c.faction.map_or("", faction_name),
            deleted_at: c
                .deleted
                .then(|| Stamp::parse(c.deleted_at.as_deref().unwrap_or_default())),
            portal_id,
            account: c.account,
            level: c.level,
            experience: c.experience,
            skill_points: c.skill_points,
            session: c.session.as_str(),
        }
    }
}

#[derive(Template)]
#[template(path = "admin_game.html")]
struct ListPage {
    nav_csrf: Option<String>,
    role: &'static str,
    is_admin: bool,
    configured: bool,
    error: Option<String>,
    /// The game server lists characters by slot (one row per character).
    slots: bool,
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
        slots: false,
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
    // By account, then live before deleted, then slot (the game server's own order).
    characters
        .sort_by(|a, b| (&a.account, a.deleted, a.slot).cmp(&(&b.account, b.deleted, b.slot)));
    (view.sanctions, view.voice_error) = sanctions(api, &actor_label(&actor)).await;
    let mut keys: Vec<String> = characters
        .iter()
        .map(|c| c.account.to_ascii_lowercase())
        .collect();
    keys.dedup();
    let ids: HashMap<String, i32> = db::account_ids_by_game_account_names(&state.pool, &keys)
        .await?
        .into_iter()
        .collect();
    view.slots = characters.iter().any(|c| c.slot.is_some());
    view.rows = characters
        .into_iter()
        .map(|c| {
            let portal_id = ids.get(&c.account.to_ascii_lowercase()).copied();
            CharacterRow::new(c, portal_id)
        })
        .collect();
    Ok(page(StatusCode::OK, view))
}

// ---- One character ----

struct CharacterView {
    /// `None` from a game server that predates character slots.
    slot: Option<u8>,
    name: String,
    faction: &'static str,
    level: u32,
    experience: i64,
    skill_points: i64,
    next_level: String,
    /// "3000 points, 1 taken, next costs 5000"; empty from a game server that predates respecs.
    respec: String,
    session: &'static str,
}

/// One of the account's characters in the character page's table.
struct SlotRow {
    slot: u8,
    name: String,
    faction: &'static str,
    level: u32,
    experience: i64,
    skill_points: i64,
    /// Empty from a game server that predates respecs.
    respec_points: String,
    /// `Some` for a deleted character ("" if the time is unknown).
    deleted_at: Option<Stamp>,
    /// The character the page shows.
    selected: bool,
}

impl SlotRow {
    fn new(c: AccountCharacter, selected: Option<u32>) -> Self {
        Self {
            selected: !c.deleted && selected == Some(c.character_id),
            deleted_at: c
                .deleted
                .then(|| Stamp::parse(c.deleted_at.as_deref().unwrap_or_default())),
            slot: c.slot,
            name: c.name,
            faction: faction_name(c.faction),
            level: c.level,
            experience: c.experience,
            skill_points: c.skill_points,
            respec_points: c.respec_points.map(|n| n.to_string()).unwrap_or_default(),
        }
    }
}

/// The character page's respec line: points, respecs taken and the next fee when known.
fn respec_summary(points: Option<i64>, taken: Option<i64>, next_fee: Option<i64>) -> String {
    let Some(points) = points else {
        return String::new();
    };
    let mut line = format!("{points} points, {} taken", taken.unwrap_or(0));
    if let Some(fee) = next_fee {
        line.push_str(&format!(", next costs {fee}"));
    }
    line
}

#[derive(Deserialize)]
struct SlotParam {
    slot: Option<String>,
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
    /// Every character of the account, deleted ones last; empty from a game server that
    /// predates character slots.
    characters: Vec<SlotRow>,
    error: Option<String>,
    sanctions: Vec<SanctionRow>,
    voice_error: Option<String>,
}

async fn character_page(
    State(state): Shared,
    headers: HeaderMap,
    Path(raw): Path<String>,
    Query(query): Query<SlotParam>,
) -> PageResult {
    let path = format!("/admin/game/characters/{raw}");
    let actor = guard!(require_staff(&state, &headers, &path, Role::Moderator).await);
    let Some(account) = account_key(&raw) else {
        return Ok(no_character(&raw.chars().take(32).collect::<String>()));
    };
    let Ok(slot) = parse_slot(query.slot.as_deref()) else {
        return Ok(bad_slot(&format!("/admin/game/characters/{account}")));
    };
    let Nav {
        nav_csrf,
        csrf,
        role,
        is_admin,
    } = actor.nav();
    let portal_id = db::account_by_game_account_name(&state.pool, &account)
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
        characters: Vec::new(),
        error: None,
        sanctions: Vec::new(),
        voice_error: None,
    };
    let Some(api) = state.settings.game_admin.as_ref() else {
        return Ok(page(StatusCode::OK, view));
    };
    view.configured = true;
    match api
        .character(&actor_label(&actor), &view.account, slot)
        .await
    {
        Ok(Some(c)) => {
            let mut all = c.characters;
            all.sort_by_key(|x| (x.deleted, x.slot));
            view.characters = all
                .into_iter()
                .map(|x| SlotRow::new(x, c.character_id))
                .collect();
            let respec = respec_summary(c.respec_points, c.respec_count, c.next_respec_fee);
            view.character = Some(CharacterView {
                respec,
                slot: c.slot,
                name: c.name.unwrap_or_default(),
                faction: c.faction.map_or("", faction_name),
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
        Ok(None) => Ok(no_character_in(&view.account, slot)),
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
    /// The slot asked for, carried into the confirmation and the links back.
    slot: Option<u8>,
    /// `?slot=N` for the links back, or empty.
    slot_query: String,
    /// The character the game server changed or would change, when it names it.
    name: Option<String>,
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
    /// The character's slot; empty or absent for an account with one character (or a game
    /// server that predates slots).
    #[serde(default)]
    slot: String,
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
    let Ok(slot) = parse_slot(Some(&form.slot)) else {
        return Ok(bad_slot(&format!("/admin/game/characters/{account}")));
    };
    let back = format!("/admin/game/characters/{account}{}", slot_query(slot));
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
        .set_level(&actor_label(&actor), &account, level, apply, slot)
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
        slot,
        slot_query: slot_query(slot),
        name: None,
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
        Err(GameAdminError::NotFound) => return Ok(no_character_in(&view.account, slot)),
        Err(GameAdminError::Refused(r)) => {
            view.refusal = Some(r.into());
            return Ok(page(StatusCode::CONFLICT, view));
        }
        Err(e @ GameAdminError::Uncertain { .. }) => {
            // Read back what the game server holds now, and keep the attempt in the audit log.
            match api
                .character(&actor_label(&actor), &view.account, slot)
                .await
            {
                Ok(Some(c)) => {
                    view.now_level = Some(c.level);
                    view.name = c.name;
                }
                Ok(None) => view.now_error = Some("the game server has no character for it".into()),
                Err(read) => view.now_error = Some(read.to_string()),
            }
            staff::log(
                &state.pool,
                actor.account(),
                "set level",
                &view.account,
                &format!(
                    "{}to {level}, unconfirmed: the game server did not confirm it",
                    character_label(slot, view.name.as_deref())
                ),
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
                "{}{}→{}, backup {}",
                character_label(change.slot.or(slot), change.name.as_deref()),
                change.from_level,
                change.to_level,
                change.backup.as_deref().unwrap_or("none")
            ),
        )
        .await?;
    }
    view.applied = change.applied;
    view.name = change.name;
    if view.slot.is_none() {
        // The game server picked the account's only character; keep to it from here.
        view.slot = change.slot;
        view.slot_query = slot_query(change.slot);
    }
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

/// "slot 1 (Name): " for the audit log, or empty when the game server names no character.
fn character_label(slot: Option<u8>, name: Option<&str>) -> String {
    match (slot, name) {
        (Some(s), Some(n)) => format!("slot {s} ({n}): "),
        (Some(s), None) => format!("slot {s}: "),
        (None, Some(n)) => format!("{n}: "),
        (None, None) => String::new(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_admin::SessionState;

    fn summary(account: &str, level: u32) -> CharacterSummary {
        CharacterSummary {
            account: account.into(),
            level,
            experience: i64::from(level) * 1000,
            skill_points: 10,
            session: SessionState::Offline,
            character_id: None,
            name: None,
            slot: None,
            faction: None,
            deleted: false,
            deleted_at: None,
        }
    }

    fn in_slot(account: &str, id: u32, name: &str, slot: u8, level: u32) -> CharacterSummary {
        CharacterSummary {
            character_id: Some(id),
            name: Some(name.into()),
            slot: Some(slot),
            faction: Some(slot),
            ..summary(account, level)
        }
    }

    fn list_page(slots: bool, rows: Vec<CharacterRow>) -> ListPage {
        ListPage {
            nav_csrf: Some("nav".into()),
            role: "admin",
            is_admin: true,
            configured: true,
            error: None,
            slots,
            rows,
            sanctions: Vec::new(),
            voice_error: None,
        }
    }

    #[test]
    fn slots_from_queries_and_forms() {
        assert_eq!(parse_slot(None), Ok(None));
        assert_eq!(parse_slot(Some("")), Ok(None));
        assert_eq!(parse_slot(Some(" 2 ")), Ok(Some(2)));
        for bad in ["3", "-1", "one", "256"] {
            assert_eq!(parse_slot(Some(bad)), Err(()), "{bad}");
        }
        assert_eq!(slot_query(None), "");
        assert_eq!(slot_query(Some(1)), "?slot=1");
        assert_eq!(character_label(Some(1), Some("Bravo")), "slot 1 (Bravo): ");
        assert_eq!(character_label(None, None), "");
    }

    #[test]
    fn list_from_an_older_game_server() {
        let html = list_page(false, vec![CharacterRow::new(summary("alpha", 5), Some(7))])
            .render()
            .unwrap();
        assert!(html.contains("<a href=\"/admin/game/characters/alpha\">alpha</a>"));
        assert!(html.contains("<th>Character</th><th>Level</th>"));
        assert!(!html.contains("<th>Slot</th>") && !html.contains("?slot="));
        assert!(!html.contains("deleted"));
        assert!(html.contains("href=\"/admin/users/7\""));
    }

    #[test]
    fn list_with_character_slots() {
        let mut gone = in_slot("alpha", 9, "AlphaOld", 1, 2);
        gone.deleted = true;
        gone.deleted_at = Some("2026-11-20T08:30:00Z".into());
        let rows = vec![
            in_slot("alpha", 11, "Alpha", 0, 5),
            in_slot("alpha", 12, "AlphaTwo", 2, 30),
            gone,
        ];
        let html = list_page(
            true,
            rows.into_iter()
                .map(|c| CharacterRow::new(c, None))
                .collect(),
        )
        .render()
        .unwrap();
        assert!(html.contains("<th>Account</th><th>Slot</th><th>Character</th><th>Faction</th>"));
        assert!(html.contains("<a href=\"/admin/game/characters/alpha?slot=0\">Alpha</a>"));
        assert!(html.contains("<a href=\"/admin/game/characters/alpha?slot=2\">AlphaTwo</a>"));
        assert!(html.contains("<td>Valor</td>") && html.contains("<td>SVER</td>"));
        // The deleted character is marked and not linked to its (free) slot.
        assert!(html.contains(
            "AlphaOld <span class=\"pill\">deleted</span> <span class=\"quiet\"><time class=\"local-time\" datetime=\"2026-11-20T08:30:00Z\" title=\"2026-11-20 08:30:00 UTC\">2026-11-20 08:30 UTC</time></span>"
        ));
        assert!(!html.contains("?slot=1"));
    }

    fn character_page(
        is_admin: bool,
        character: CharacterView,
        characters: Vec<SlotRow>,
    ) -> CharacterPage {
        CharacterPage {
            nav_csrf: Some("nav".into()),
            csrf: "form-token".into(),
            role: if is_admin { "admin" } else { "moderator" },
            is_admin,
            configured: true,
            account: "alpha".into(),
            portal_id: None,
            character: Some(character),
            characters,
            error: None,
            sanctions: Vec::new(),
            voice_error: None,
        }
    }

    fn view(slot: Option<u8>, name: &str, faction: &'static str) -> CharacterView {
        CharacterView {
            slot,
            name: name.into(),
            faction,
            level: 30,
            experience: 9000,
            skill_points: 40,
            next_level: "9500".into(),
            respec: "3000 points, 1 taken, next costs 5000".into(),
            session: "offline",
        }
    }

    fn account_character(id: u32, name: &str, slot: u8, deleted: bool) -> AccountCharacter {
        AccountCharacter {
            character_id: id,
            name: name.into(),
            slot,
            faction: slot,
            level: 5 + u32::from(slot),
            experience: 100,
            skill_points: 2,
            deleted,
            deleted_at: deleted.then(|| "2026-11-20T08:30:00Z".into()),
            respec_points: Some(3000 + i64::from(slot)),
        }
    }

    #[test]
    fn character_page_with_several_characters() {
        let rows = [
            account_character(11, "Alpha", 0, false),
            account_character(12, "AlphaTwo", 2, false),
            account_character(9, "AlphaOld", 1, true),
        ]
        .into_iter()
        .map(|c| SlotRow::new(c, Some(12)))
        .collect();
        let html = character_page(true, view(Some(2), "AlphaTwo", "SVER"), rows)
            .render()
            .unwrap();
        assert!(html.contains("<h2>Characters</h2>"));
        // Another live character links to its slot; the one shown is marked, not linked.
        assert!(html.contains("<a href=\"/admin/game/characters/alpha?slot=0\">Alpha</a>"));
        assert!(html.contains("<strong>AlphaTwo</strong> <span class=\"quiet\">(shown below)"));
        assert!(!html.contains("?slot=2\">"));
        // The deleted one shows when, and has no link.
        assert!(html.contains(
            "<span class=\"pill\">deleted</span> <time class=\"local-time\" datetime=\"2026-11-20T08:30:00Z\" title=\"2026-11-20 08:30:00 UTC\">2026-11-20 08:30 UTC</time>"
        ));
        assert!(!html.contains("?slot=1"));
        assert!(html.contains("<h2>Slot 2 · AlphaTwo</h2>"));
        assert!(html.contains("<dt>Faction</dt><dd>SVER</dd>"));
        // Each slot's respec points, and the shown character's respec line.
        assert!(html.contains("<th>Respec points</th>") && html.contains("<td>3002</td>"));
        assert!(html.contains("<dt>Respec</dt><dd>3000 points, 1 taken, next costs 5000</dd>"));
        // The level form names the slot it changes.
        assert!(html.contains("in slot 2 (AlphaTwo)"));
        assert!(html.contains("<input type=\"hidden\" name=\"slot\" value=\"2\">"));

        // Moderators see the characters, without the level form.
        let rows = vec![SlotRow::new(
            account_character(11, "Alpha", 0, false),
            Some(11),
        )];
        let html = character_page(false, view(Some(0), "Alpha", "Valor"), rows)
            .render()
            .unwrap();
        assert!(html.contains("<strong>Alpha</strong>") && !html.contains("name=\"slot\""));
    }

    #[test]
    fn character_page_from_an_older_game_server() {
        let html = character_page(true, view(None, "", ""), Vec::new())
            .render()
            .unwrap();
        assert!(!html.contains("<h2>Characters</h2>") && !html.contains("<h2>Slot"));
        assert!(!html.contains("Faction") && !html.contains("name=\"slot\""));
        assert!(html.contains("Raises the character to a level"));
        assert!(html.contains("action=\"/admin/game/characters/alpha/level\""));
    }

    fn level_page(slot: Option<u8>) -> LevelPage {
        LevelPage {
            nav_csrf: Some("nav".into()),
            csrf: "form-token".into(),
            role: "admin",
            is_admin: true,
            account: "alpha".into(),
            level: 20,
            slot,
            slot_query: slot_query(slot),
            name: slot.map(|_| "AlphaTwo".into()),
            applied: false,
            change: None,
            refusal: None,
            error: None,
            uncertain: None,
            now_level: None,
            now_error: None,
        }
    }

    #[test]
    fn level_pages_keep_the_slot() {
        let mut planned = level_page(Some(2));
        planned.change = Some(ChangeView {
            from_level: 5,
            to_level: 20,
            experience_before: 5000,
            experience_after: 20000,
            skill_points_before: 10,
            skill_points_after: 40,
            backup: None,
        });
        let html = planned.render().unwrap();
        assert!(html.contains("Character in slot 2: AlphaTwo."));
        assert!(html.contains("<input type=\"hidden\" name=\"slot\" value=\"2\">"));
        assert!(html.contains("href=\"/admin/game/characters/alpha?slot=2\">Cancel"));
        assert!(html.contains("href=\"/admin/game/characters/alpha?slot=2\">Back to alpha"));

        // An account with several characters and no slot asked: the refusal says what to do.
        let mut refused = level_page(None);
        refused.refusal = Some(
            Refusal {
                code: "ambiguous_character".into(),
                message: "alpha has 2 live characters; name a slot".into(),
            }
            .into(),
        );
        let html = refused.render().unwrap();
        assert!(html.contains("ambiguous_character") && html.contains("choose the character"));
        assert!(html.contains("href=\"/admin/game/characters/alpha\">Back to alpha"));
        assert!(!html.contains("name=\"slot\"") && !html.contains("Character in slot"));
    }
}
