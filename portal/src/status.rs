//! The public server status page (`/status`) and its JSON (`/api/v1/status`), and the
//! maintenance banner staff set from the admin panel.
//!
//! A background task asks each configured game server's admin API for its status every
//! `PORTAL_STATUS_REFRESH_SECONDS` and keeps the answers in memory; requests only read that
//! cache, so public traffic never reaches a game server. The game server reports counts only,
//! and the page shows only the fields copied into [`StatusView`]: never an account or player
//! name. See `docs/portal-status.md`.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{Context, bail};
use askama::Template;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Serialize;
use sqlx::PgPool;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::game_admin::{GameAdmin, GameAdminError, ServerStatus};
use crate::web::{AppState, PageResult, Shared, current_session, page, when};

/// The actor the game server logs for status requests.
pub const ACTOR: &str = "portal status page";
/// Default seconds between polls.
pub const DEFAULT_REFRESH_SECONDS: u64 = 15;
/// Longest banner message.
pub const MAX_BANNER: usize = 500;
/// Seconds the status page waits before reloading itself.
pub const PAGE_REFRESH_SECONDS: u64 = 15;

/// One game server shown on the status page.
#[derive(Debug, Clone)]
pub struct StatusServer {
    /// Short identifier (`live`, `modes`): lowercase letters, digits and `-`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// The host name players connect to, shown under the name.
    pub host: Option<String>,
    /// Shown only on the admin panel's status page.
    pub staff_only: bool,
    pub admin: GameAdmin,
}

fn valid_id(id: &str) -> bool {
    (1..=32).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The servers to show, from `PORTAL_STATUS_SERVERS` (comma-separated ids, in order) and each
/// id's `PORTAL_STATUS_<ID>_*` settings, read through `env`. `live` falls back to the admin
/// panel's game server (`game_admin`). With `PORTAL_STATUS_SERVERS` unset, the page shows
/// `live` when the admin panel has a game server, and nothing otherwise.
pub fn servers_from_env(
    game_admin: Option<&GameAdmin>,
    env: impl Fn(&str) -> Option<String>,
) -> anyhow::Result<Vec<StatusServer>> {
    let get = |name: &str| {
        env(name)
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    };
    let list = match get("PORTAL_STATUS_SERVERS") {
        Some(list) => list,
        None if game_admin.is_some() => "live".to_owned(),
        None => return Ok(Vec::new()),
    };
    let mut servers: Vec<StatusServer> = Vec::new();
    for id in list.split(',').map(str::trim).filter(|i| !i.is_empty()) {
        if !valid_id(id) {
            bail!(
                "PORTAL_STATUS_SERVERS entry {id:?} must be 1 to 32 lowercase letters, digits or -"
            );
        }
        if servers.iter().any(|s| s.id == id) {
            bail!("PORTAL_STATUS_SERVERS names {id} twice");
        }
        let prefix = format!(
            "PORTAL_STATUS_{}_",
            id.to_ascii_uppercase().replace('-', "_")
        );
        let var = |suffix: &str| get(&format!("{prefix}{suffix}"));
        let url = var("ADMIN_URL");
        let secret = var("ADMIN_SECRET_FILE");
        let admin = match (id, game_admin, url, secret) {
            ("live", Some(api), None, None) => api.clone(),
            (_, _, url, secret) => GameAdmin::from_config(
                url.as_deref(),
                secret.as_deref().map(std::path::Path::new),
            )
            .with_context(|| format!("status server {id}"))?
            .with_context(|| {
                format!("status server {id} needs {prefix}ADMIN_URL and {prefix}ADMIN_SECRET_FILE")
            })?,
        };
        let staff_only = match var("STAFF_ONLY").as_deref() {
            None | Some("false") => false,
            Some("true") => true,
            Some(_) => bail!("{prefix}STAFF_ONLY must be true or false"),
        };
        let name = var("NAME").unwrap_or_else(|| match id {
            "live" => "Live".to_owned(),
            other => other.to_owned(),
        });
        servers.push(StatusServer {
            id: id.to_owned(),
            name: shown(&name, 64),
            host: var("HOST").map(|h| shown(&h, 253)),
            staff_only,
            admin,
        });
    }
    Ok(servers)
}

/// Printable text of at most `max` characters, for values shown on the page.
fn shown(text: &str, max: usize) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect::<String>()
        .trim()
        .to_owned()
}

// ---- The cache ----

/// What the last polls of one server found.
#[derive(Debug, Clone, Default)]
pub struct Polled {
    /// The last successful answer and when it came.
    pub ok: Option<(OffsetDateTime, ServerStatus)>,
    /// When the last poll finished.
    pub checked: Option<OffsetDateTime>,
    /// Why the last poll failed, if it did (shown publicly: never a secret or URL).
    pub error: Option<&'static str>,
}

/// The maintenance or incident notice, set by staff.
#[derive(Debug, Clone, PartialEq)]
pub struct Banner {
    pub message: String,
    pub restart_expected_at: Option<OffsetDateTime>,
    pub updated_at: OffsetDateTime,
}

#[derive(Default)]
struct Cache {
    polled: Vec<Polled>,
    banner: Option<Banner>,
}

pub struct StatusService {
    servers: Vec<StatusServer>,
    refresh: Duration,
    cache: RwLock<Cache>,
}

impl StatusService {
    pub fn new(servers: Vec<StatusServer>, refresh: Duration) -> Self {
        let polled = vec![Polled::default(); servers.len()];
        Self {
            servers,
            refresh,
            cache: RwLock::new(Cache {
                polled,
                banner: None,
            }),
        }
    }

    pub fn servers(&self) -> &[StatusServer] {
        &self.servers
    }

    /// A server counts as down once its last good answer is older than this.
    pub fn stale_after(&self) -> Duration {
        self.refresh * 3 + Duration::from_secs(5)
    }

    /// Polls every server once, concurrently, and reloads the banner.
    pub async fn refresh(&self, pool: &PgPool) {
        let mut polls = tokio::task::JoinSet::new();
        for (index, server) in self.servers.iter().enumerate() {
            let admin = server.admin.clone();
            polls.spawn(async move { (index, admin.status_quietly(ACTOR).await) });
        }
        while let Some(joined) = polls.join_next().await {
            let Ok((index, result)) = joined else {
                continue;
            };
            self.record(index, result, OffsetDateTime::now_utc());
        }
        match load_banner(pool).await {
            Ok(banner) => self.write().banner = banner,
            Err(e) => tracing::warn!(error = %e, "could not reload the status banner"),
        }
    }

    /// Stores one poll's outcome, logging only when a server goes down or comes back.
    pub fn record(
        &self,
        index: usize,
        result: Result<ServerStatus, GameAdminError>,
        now: OffsetDateTime,
    ) {
        let Some(server) = self.servers.get(index) else {
            return;
        };
        let mut cache = self.write();
        let polled = &mut cache.polled[index];
        let was_failing = polled.error.is_some() || polled.checked.is_none();
        polled.checked = Some(now);
        match result {
            Ok(status) => {
                if was_failing {
                    tracing::info!(server = %server.id, "status: game server answers");
                }
                polled.ok = Some((now, status));
                polled.error = None;
            }
            Err(e) => {
                let reason = failure_reason(&e);
                if polled.error != Some(reason) {
                    tracing::warn!(server = %server.id, error = %e, "status: game server poll failed");
                }
                polled.error = Some(reason);
            }
        }
    }

    /// Polls now and then every refresh interval, for the life of the process.
    pub fn spawn(self: &Arc<Self>, pool: PgPool) {
        let service = self.clone();
        tokio::spawn(async move {
            let mut every = tokio::time::interval(service.refresh);
            every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                every.tick().await;
                service.refresh(&pool).await;
            }
        });
    }

    pub fn banner(&self) -> Option<Banner> {
        self.read().banner.clone()
    }

    /// Replaces the cached banner after staff changed it.
    pub fn set_banner(&self, banner: Option<Banner>) {
        self.write().banner = banner;
    }

    /// The page's contents: public servers, or every server for staff.
    pub fn view(&self, now: OffsetDateTime, staff: bool) -> StatusView {
        let cache = self.read();
        let servers: Vec<ServerView> = self
            .servers
            .iter()
            .zip(&cache.polled)
            .filter(|(server, _)| staff || !server.staff_only)
            .map(|(server, polled)| server_view(server, polled, now, self.stale_after()))
            .collect();
        StatusView {
            generated_at: rfc3339(now),
            state: overall(servers.iter().map(|s| s.state)).as_str(),
            banner: cache.banner.as_ref().map(BannerView::from),
            refresh_seconds: self.refresh.as_secs(),
            servers,
        }
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Cache> {
        self.cache.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Cache> {
        self.cache.write().unwrap_or_else(|e| e.into_inner())
    }
}

/// A short, public reason for a failed poll.
fn failure_reason(e: &GameAdminError) -> &'static str {
    match e {
        GameAdminError::Unreachable { .. } => "The game server did not answer.",
        GameAdminError::NotFound => "This game server version does not report its status.",
        GameAdminError::Unauthorized => {
            "The portal's credentials for this game server were refused."
        }
        _ => "The game server gave an unexpected answer.",
    }
}

// ---- Aggregation ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Health {
    Up,
    Degraded,
    Down,
    /// Not polled yet.
    Unknown,
}

impl Health {
    pub fn as_str(self) -> &'static str {
        match self {
            Health::Up => "up",
            Health::Degraded => "degraded",
            Health::Down => "down",
            Health::Unknown => "unknown",
        }
    }
}

/// The site-wide state: up only when every server is up, down only when every one is down.
pub fn overall(states: impl IntoIterator<Item = Health>) -> Health {
    let states: Vec<Health> = states.into_iter().collect();
    if states.is_empty() || states.iter().all(|s| *s == Health::Unknown) {
        Health::Unknown
    } else if states.iter().all(|s| *s == Health::Up) {
        Health::Up
    } else if states.iter().all(|s| *s == Health::Down) {
        Health::Down
    } else {
        Health::Degraded
    }
}

/// A server's state: down when its last good answer is older than `stale_after` (or it never
/// answered), else up when every world admits players, degraded when some do, down when none do.
pub fn server_state(polled: &Polled, now: OffsetDateTime, stale_after: Duration) -> Health {
    let Some((at, status)) = &polled.ok else {
        return if polled.checked.is_none() {
            Health::Unknown
        } else {
            Health::Down
        };
    };
    if now - *at > stale_after {
        return Health::Down;
    }
    let open = status.worlds.iter().filter(|w| w.state == "open").count();
    if open == 0 {
        Health::Down
    } else if open == status.worlds.len() {
        Health::Up
    } else {
        Health::Degraded
    }
}

pub fn mode_name(game_type: u8) -> &'static str {
    match game_type {
        0 => "Sabotage",
        1 => "Acquisition",
        2 => "Domination",
        _ => "Unknown mode",
    }
}

pub fn faction_name(faction: u8) -> &'static str {
    match faction {
        0 => "Valor",
        1 => "Raven",
        2 => "SVER",
        _ => "Unknown",
    }
}

fn world_state_text(state: &str) -> &'static str {
    match state {
        "open" => "Open",
        "draining" => "Closed to new players",
        "faulted" => "Down",
        "unavailable" => "Unavailable",
        _ => "Unknown",
    }
}

fn rfc3339(t: OffsetDateTime) -> String {
    t.format(&Rfc3339).unwrap_or_default()
}

/// `3 d 4 h`, `2 h 5 min` or `45 min`.
pub fn duration_text(seconds: u64) -> String {
    let (days, hours, minutes) = (seconds / 86_400, seconds / 3600 % 24, seconds / 60 % 60);
    if days > 0 {
        format!("{days} d {hours} h")
    } else if hours > 0 {
        format!("{hours} h {minutes} min")
    } else {
        format!("{minutes} min")
    }
}

// ---- What the page and the JSON show ----

#[derive(Debug, Serialize)]
pub struct StatusView {
    pub generated_at: String,
    /// `up`, `degraded`, `down` or `unknown` across the servers shown.
    pub state: &'static str,
    pub banner: Option<BannerView>,
    pub refresh_seconds: u64,
    pub servers: Vec<ServerView>,
}

#[derive(Debug, Serialize)]
pub struct BannerView {
    pub message: String,
    pub restart_expected_at: Option<String>,
    #[serde(skip)]
    pub restart_text: Option<String>,
    pub updated_at: String,
}

impl From<&Banner> for BannerView {
    fn from(b: &Banner) -> Self {
        Self {
            message: b.message.clone(),
            restart_expected_at: b.restart_expected_at.map(rfc3339),
            restart_text: b.restart_expected_at.map(when),
            updated_at: rfc3339(b.updated_at),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ServerView {
    pub id: String,
    pub name: String,
    pub host: Option<String>,
    #[serde(serialize_with = "state_str")]
    pub state: Health,
    /// Why, in a sentence.
    pub summary: String,
    /// When the server last answered.
    pub last_seen: Option<String>,
    #[serde(skip)]
    pub last_seen_text: Option<String>,
    pub medius_version: Option<String>,
    pub gameplay_version: Option<String>,
    pub started_at: Option<String>,
    pub uptime_seconds: Option<u64>,
    #[serde(skip)]
    pub uptime_text: Option<String>,
    pub players_online: Option<u32>,
    pub queued: Option<u32>,
    pub dme_ports: Option<PortsView>,
    pub worlds: Vec<WorldView>,
}

fn state_str<S: serde::Serializer>(state: &Health, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(state.as_str())
}

#[derive(Debug, Serialize)]
pub struct PortsView {
    pub in_use: u32,
    pub capacity: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct WorldView {
    pub id: String,
    /// `open`, `draining`, `faulted` or `unavailable`.
    pub state: String,
    #[serde(skip)]
    pub state_text: &'static str,
    pub open: bool,
    pub modes: Vec<&'static str>,
    /// Humans waiting for this world: those placed on it, plus the global queue for the modes
    /// only this world serves.
    pub queued: u32,
    pub human_capacity: u32,
    #[serde(rename = "match")]
    pub current: Option<MatchView>,
}

#[derive(Debug, Serialize)]
pub struct MatchView {
    pub profile: String,
    pub mode: &'static str,
    pub attackers: &'static str,
    pub defenders: &'static str,
    pub humans: u32,
    pub attacking_humans: u32,
    pub defending_humans: u32,
    pub human_capacity: u32,
    pub bots: u32,
}

/// One server for the page. Only these fields are copied from the game server's answer.
pub fn server_view(
    server: &StatusServer,
    polled: &Polled,
    now: OffsetDateTime,
    stale_after: Duration,
) -> ServerView {
    let state = server_state(polled, now, stale_after);
    let last_seen = polled.ok.as_ref().map(|(at, _)| *at);
    let mut view = ServerView {
        id: server.id.clone(),
        name: server.name.clone(),
        host: server.host.clone(),
        state,
        summary: String::new(),
        last_seen: last_seen.map(rfc3339),
        last_seen_text: last_seen.map(when),
        medius_version: None,
        gameplay_version: None,
        started_at: None,
        uptime_seconds: None,
        uptime_text: None,
        players_online: None,
        queued: None,
        dme_ports: None,
        worlds: Vec::new(),
    };
    let fresh = polled
        .ok
        .as_ref()
        .filter(|(at, _)| now - *at <= stale_after)
        .map(|(_, status)| status);
    let Some(status) = fresh else {
        view.summary = match (state, polled.error) {
            (Health::Unknown, _) => "Checking the game server.".to_owned(),
            (_, Some(reason)) => reason.to_owned(),
            _ => "The game server has not answered recently.".to_owned(),
        };
        return view;
    };
    let worlds = &status.worlds;
    let served_by = |game_type: u8| {
        worlds
            .iter()
            .filter(|w| w.game_types.contains(&game_type))
            .count()
    };
    view.worlds = worlds
        .iter()
        .map(|w| {
            let mut modes: Vec<&'static str> = w.game_types.iter().map(|t| mode_name(*t)).collect();
            modes.dedup();
            let global: u32 = w
                .game_types
                .iter()
                .filter(|t| served_by(**t) == 1)
                .filter_map(|t| status.queued_by_game_type.get(usize::from(*t)))
                .sum();
            WorldView {
                id: shown(&w.worker_id, 64),
                state: shown(&w.state, 16),
                state_text: world_state_text(&w.state),
                open: w.state == "open",
                modes,
                queued: w.pending.saturating_add(global),
                human_capacity: w.human_capacity,
                current: w.current.as_ref().map(|m| MatchView {
                    profile: shown(&m.profile_id, 64),
                    mode: mode_name(m.game_type),
                    attackers: faction_name(m.attacking_faction),
                    defenders: faction_name(m.defending_faction),
                    humans: m.humans,
                    attacking_humans: m.attacking_humans,
                    defending_humans: m.defending_humans,
                    human_capacity: m.human_capacity,
                    bots: m.bots,
                }),
            }
        })
        .collect();
    let open = view.worlds.iter().filter(|w| w.open).count();
    view.summary = match (worlds.len(), open) {
        (0, _) => "The game server runs no worlds.".to_owned(),
        (n, o) if o == n => {
            if n == 1 {
                "The world is open.".to_owned()
            } else {
                format!("All {n} worlds are open.")
            }
        }
        (n, 0) => {
            if n == 1 {
                "The world is closed.".to_owned()
            } else {
                format!("All {n} worlds are closed.")
            }
        }
        (n, o) => format!("{} of {n} worlds closed.", n - o),
    };
    view.medius_version = status.medius_version.as_deref().map(|v| shown(v, 32));
    view.gameplay_version = status.gameplay_version.as_deref().map(|v| shown(v, 32));
    view.started_at = status
        .started_at
        .as_deref()
        .and_then(|t| OffsetDateTime::parse(t, &Rfc3339).ok())
        .map(rfc3339);
    view.uptime_seconds = Some(status.uptime_seconds);
    view.uptime_text = Some(duration_text(status.uptime_seconds));
    view.players_online = status.players_online;
    view.queued = Some(status.queued);
    view.dme_ports = Some(PortsView {
        in_use: status.dme_ports.in_use,
        capacity: status.dme_ports.capacity,
    });
    view
}

// ---- The banner in the database ----

pub async fn load_banner(pool: &PgPool) -> sqlx::Result<Option<Banner>> {
    let row: Option<(String, Option<OffsetDateTime>, OffsetDateTime)> = sqlx::query_as(
        "SELECT message, restart_expected_at, updated_at FROM status_banner WHERE id = 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(
        row.map(|(message, restart_expected_at, updated_at)| Banner {
            message,
            restart_expected_at,
            updated_at,
        }),
    )
}

pub async fn save_banner(
    pool: &PgPool,
    message: &str,
    restart_expected_at: Option<OffsetDateTime>,
    updated_by: &str,
) -> sqlx::Result<Banner> {
    let (message, restart_expected_at, updated_at): (
        String,
        Option<OffsetDateTime>,
        OffsetDateTime,
    ) = sqlx::query_as(
        "INSERT INTO status_banner (id, message, restart_expected_at, updated_by, updated_at)
             VALUES (1, $1, $2, $3, now())
             ON CONFLICT (id) DO UPDATE SET message = EXCLUDED.message,
                 restart_expected_at = EXCLUDED.restart_expected_at,
                 updated_by = EXCLUDED.updated_by, updated_at = EXCLUDED.updated_at
             RETURNING message, restart_expected_at, updated_at",
    )
    .bind(message)
    .bind(restart_expected_at)
    .bind(updated_by)
    .fetch_one(pool)
    .await?;
    Ok(Banner {
        message,
        restart_expected_at,
        updated_at,
    })
}

/// Whether there was a banner to remove.
pub async fn clear_banner(pool: &PgPool) -> sqlx::Result<bool> {
    Ok(sqlx::query("DELETE FROM status_banner WHERE id = 1")
        .execute(pool)
        .await?
        .rows_affected()
        > 0)
}

/// A banner message as staff typed it: trimmed, line breaks become spaces, other control
/// characters are dropped. `None` when empty or longer than [`MAX_BANNER`].
pub fn banner_message(raw: &str) -> Option<String> {
    let text: String = raw
        .chars()
        .map(|c| {
            if c == '\n' || c == '\r' || c == '\t' {
                ' '
            } else {
                c
            }
        })
        .filter(|c| !c.is_control())
        .collect();
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!text.is_empty() && text.chars().count() <= MAX_BANNER).then_some(text)
}

/// A `datetime-local` form value (`2026-11-26T18:30`, seconds optional), read as UTC.
/// `Ok(None)` for an empty value.
pub fn restart_time(raw: &str) -> Result<Option<OffsetDateTime>, time::error::Parse> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let minutes = time::macros::format_description!("[year]-[month]-[day]T[hour]:[minute]");
    let seconds =
        time::macros::format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]");
    time::PrimitiveDateTime::parse(raw, &minutes)
        .or_else(|_| time::PrimitiveDateTime::parse(raw, &seconds))
        .map(|t| Some(t.assume_utc()))
}

// ---- Public routes ----

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/status", get(status_page))
        .route("/api/v1/status", get(status_json))
}

#[derive(Template)]
#[template(path = "status.html")]
struct StatusPage {
    nav_csrf: Option<String>,
    view: StatusView,
    configured: bool,
}

async fn status_page(State(state): Shared, headers: HeaderMap) -> PageResult {
    // Only a signed-in visitor (with a session cookie) costs a database read.
    let nav_csrf = current_session(&state, &headers)
        .await?
        .map(|s| s.csrf_token);
    let view = state.status.view(OffsetDateTime::now_utc(), false);
    Ok(page(
        StatusCode::OK,
        StatusPage {
            nav_csrf,
            configured: !view.servers.is_empty(),
            view,
        },
    ))
}

async fn status_json(State(state): Shared) -> Response {
    let view = state.status.view(OffsetDateTime::now_utc(), false);
    let mut response = axum::Json(view).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=5"),
    );
    response
}

/// The status page's heading for the overall state.
pub fn headline(state: &str) -> &'static str {
    match state {
        "up" => "All servers are up",
        "degraded" => "Some servers or worlds are closed",
        "down" => "The game servers are down",
        _ => "Checking the game servers",
    }
}

/// A server state's label.
pub fn state_label(state: &str) -> &'static str {
    match state {
        "up" => "Up",
        "degraded" => "Degraded",
        "down" => "Down",
        _ => "Checking",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_admin::{DmePorts, MatchStatus, WorldStatus};

    const SECRET: &[u8] = b"0123456789abcdef0123456789abcdef";

    fn api() -> GameAdmin {
        GameAdmin::new("http://medius.example.test:10490", SECRET.to_vec()).unwrap()
    }

    fn world(id: &str, state: &str, game_type: u8, running: bool) -> WorldStatus {
        WorldStatus {
            worker_id: id.into(),
            state: state.into(),
            game_types: vec![game_type],
            human_capacity: 32,
            pending: 1,
            current: running.then(|| MatchStatus {
                profile_id: format!("synthetic-{id}"),
                game_type,
                attacking_faction: 1,
                defending_faction: 2,
                humans: 5,
                attacking_humans: 3,
                defending_humans: 2,
                human_capacity: 32,
                bots: 59,
                world_capacity: 64,
            }),
        }
    }

    fn status(worlds: Vec<WorldStatus>) -> ServerStatus {
        ServerStatus {
            medius_version: Some("0.12.0".into()),
            gameplay_version: Some("0.16.0".into()),
            started_at: Some("2026-11-26T10:00:00Z".into()),
            uptime_seconds: 93_780,
            mode: "registry".into(),
            players_online: Some(7),
            queued: 3,
            queued_by_game_type: vec![2, 1, 0],
            dme_ports: DmePorts {
                capacity: Some(32),
                in_use: 5,
            },
            worlds,
        }
    }

    fn server(id: &str, staff_only: bool) -> StatusServer {
        StatusServer {
            id: id.into(),
            name: id.to_uppercase(),
            host: Some(format!("{id}.example.test")),
            staff_only,
            admin: api(),
        }
    }

    fn at(seconds: i64) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_800_000_000 + seconds).unwrap()
    }

    const STALE: Duration = Duration::from_secs(50);

    #[test]
    fn up_degraded_down() {
        let ok = |worlds| Polled {
            ok: Some((at(0), status(worlds))),
            checked: Some(at(0)),
            error: None,
        };
        let all_open = ok(vec![
            world("sabotage", "open", 0, true),
            world("acquisition", "open", 1, false),
        ]);
        assert_eq!(server_state(&all_open, at(10), STALE), Health::Up);
        let one_closed = ok(vec![
            world("sabotage", "open", 0, true),
            world("acquisition", "faulted", 1, false),
            world("domination", "draining", 2, true),
        ]);
        assert_eq!(server_state(&one_closed, at(10), STALE), Health::Degraded);
        let none_open = ok(vec![world("sabotage", "unavailable", 0, false)]);
        assert_eq!(server_state(&none_open, at(10), STALE), Health::Down);
        assert_eq!(server_state(&ok(vec![]), at(10), STALE), Health::Down);
        // Never polled, never answered, and an answer gone stale.
        assert_eq!(
            server_state(&Polled::default(), at(0), STALE),
            Health::Unknown
        );
        let unreachable = Polled {
            ok: None,
            checked: Some(at(0)),
            error: Some("The game server did not answer."),
        };
        assert_eq!(server_state(&unreachable, at(0), STALE), Health::Down);
        assert_eq!(server_state(&all_open, at(51), STALE), Health::Down);
        // One failed poll between good ones keeps the last answer while it is fresh.
        let mut flaky = all_open.clone();
        flaky.checked = Some(at(15));
        flaky.error = Some("The game server did not answer.");
        assert_eq!(server_state(&flaky, at(16), STALE), Health::Up);

        assert_eq!(overall([Health::Up, Health::Up]), Health::Up);
        assert_eq!(overall([Health::Up, Health::Down]), Health::Degraded);
        assert_eq!(overall([Health::Down, Health::Down]), Health::Down);
        assert_eq!(overall([Health::Unknown]), Health::Unknown);
        assert_eq!(overall([]), Health::Unknown);
        assert_eq!(overall([Health::Up, Health::Unknown]), Health::Degraded);
    }

    #[test]
    fn views_copy_counts_only() {
        let polled = Polled {
            ok: Some((
                at(0),
                status(vec![
                    world("sabotage", "open", 0, true),
                    world("acquisition", "faulted", 1, false),
                ]),
            )),
            checked: Some(at(0)),
            error: None,
        };
        let view = server_view(&server("live", false), &polled, at(5), STALE);
        assert_eq!(view.state, Health::Degraded);
        assert_eq!(view.summary, "1 of 2 worlds closed.");
        assert_eq!(view.players_online, Some(7));
        assert_eq!(view.queued, Some(3));
        assert_eq!(view.uptime_text.as_deref(), Some("1 d 2 h"));
        assert_eq!(view.started_at.as_deref(), Some("2026-11-26T10:00:00Z"));
        let sabotage = &view.worlds[0];
        assert!(sabotage.open);
        assert_eq!(sabotage.modes, vec!["Sabotage"]);
        // One placed on the world plus the two waiting for the mode only it serves.
        assert_eq!(sabotage.queued, 3);
        let m = sabotage.current.as_ref().unwrap();
        assert_eq!(
            (m.mode, m.attackers, m.defenders),
            ("Sabotage", "Raven", "SVER")
        );
        assert_eq!((m.humans, m.human_capacity, m.bots), (5, 32, 59));
        assert_eq!(view.worlds[1].state_text, "Down");

        // Down: the last answer's time stays, its counts do not.
        let down = server_view(&server("live", false), &polled, at(60), STALE);
        assert_eq!(down.state, Health::Down);
        assert!(down.worlds.is_empty() && down.players_online.is_none());
        assert_eq!(down.last_seen.as_deref(), Some("2027-01-15T08:00:00Z"));
    }

    #[test]
    fn staff_only_servers_and_the_cache() {
        let service = StatusService::new(
            vec![server("live", false), server("test", true)],
            Duration::from_secs(15),
        );
        assert_eq!(service.view(at(0), false).state, "unknown");
        service.record(
            0,
            Ok(status(vec![world("sabotage", "open", 0, false)])),
            at(0),
        );
        service.record(
            1,
            Err(GameAdminError::Unreachable {
                url: "http://medius.example.test:10490".into(),
                reason: "the connection failed",
            }),
            at(0),
        );
        let public = service.view(at(1), false);
        assert_eq!(public.servers.len(), 1);
        assert_eq!(public.state, "up");
        let staff = service.view(at(1), true);
        assert_eq!(staff.servers.len(), 2);
        assert_eq!(staff.state, "degraded");
        assert_eq!(staff.servers[1].summary, "The game server did not answer.");
        let json = serde_json::to_string(&staff).unwrap();
        assert!(!json.contains("medius.example.test:10490"), "{json}");

        service.set_banner(Some(Banner {
            message: "Restarting for an update".into(),
            restart_expected_at: Some(at(600)),
            updated_at: at(0),
        }));
        let banner = service.view(at(1), false).banner.unwrap();
        assert_eq!(banner.message, "Restarting for an update");
        assert_eq!(
            banner.restart_expected_at.as_deref(),
            Some("2027-01-15T08:10:00Z")
        );
        service.set_banner(None);
        assert!(service.view(at(1), false).banner.is_none());
    }

    #[test]
    fn configuration() {
        let dir = std::env::temp_dir().join(format!(
            "portal-status-{}",
            crate::tokens::b64url(&crate::tokens::random_bytes::<9>())
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let secret = dir.join("modes-secret");
        std::fs::write(&secret, SECRET).unwrap();
        let secret = secret.to_str().unwrap().to_owned();
        let env = |pairs: &[(&str, &str)]| {
            let pairs: Vec<(String, String)> = pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect();
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| k == name)
                    .map(|(_, v)| v.clone())
            }
        };
        let live = api();
        // Nothing configured: live when the admin panel has a game server, else nothing.
        let default = servers_from_env(Some(&live), env(&[])).unwrap();
        assert_eq!(default.len(), 1);
        assert_eq!(
            (default[0].id.as_str(), default[0].name.as_str()),
            ("live", "Live")
        );
        assert!(servers_from_env(None, env(&[])).unwrap().is_empty());

        let servers = servers_from_env(
            Some(&live),
            env(&[
                ("PORTAL_STATUS_SERVERS", "live, modes,test-lab"),
                ("PORTAL_STATUS_LIVE_HOST", "play.example.test"),
                ("PORTAL_STATUS_MODES_NAME", "Modes"),
                (
                    "PORTAL_STATUS_MODES_ADMIN_URL",
                    "http://modes.example.test:10490",
                ),
                ("PORTAL_STATUS_MODES_ADMIN_SECRET_FILE", &secret),
                (
                    "PORTAL_STATUS_TEST_LAB_ADMIN_URL",
                    "http://lab.example.test:10490",
                ),
                ("PORTAL_STATUS_TEST_LAB_ADMIN_SECRET_FILE", &secret),
                ("PORTAL_STATUS_TEST_LAB_STAFF_ONLY", "true"),
            ]),
        )
        .unwrap();
        let ids: Vec<&str> = servers.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["live", "modes", "test-lab"]);
        assert_eq!(servers[0].host.as_deref(), Some("play.example.test"));
        assert_eq!(servers[1].name, "Modes");
        assert_eq!(
            servers[1].admin.base_url(),
            "http://modes.example.test:10490"
        );
        assert!(!servers[1].staff_only && servers[2].staff_only);

        for bad in [
            vec![("PORTAL_STATUS_SERVERS", "Live")],
            vec![("PORTAL_STATUS_SERVERS", "live,live")],
            vec![("PORTAL_STATUS_SERVERS", "modes")],
            vec![
                ("PORTAL_STATUS_SERVERS", "modes"),
                (
                    "PORTAL_STATUS_MODES_ADMIN_URL",
                    "http://modes.example.test:10490",
                ),
            ],
            vec![
                ("PORTAL_STATUS_SERVERS", "live"),
                ("PORTAL_STATUS_LIVE_STAFF_ONLY", "yes"),
            ],
        ] {
            assert!(servers_from_env(Some(&live), env(&bad)).is_err(), "{bad:?}");
        }
        // Without the admin panel's game server, live needs its own settings.
        assert!(servers_from_env(None, env(&[("PORTAL_STATUS_SERVERS", "live")])).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn banner_input() {
        assert_eq!(
            banner_message("  Restart at\n18:00\tUTC \u{7} ").as_deref(),
            Some("Restart at 18:00 UTC")
        );
        assert_eq!(banner_message("   "), None);
        assert_eq!(
            banner_message(&"a".repeat(MAX_BANNER)).map(|m| m.len()),
            Some(MAX_BANNER)
        );
        assert_eq!(banner_message(&"a".repeat(MAX_BANNER + 1)), None);
        assert_eq!(restart_time("").unwrap(), None);
        assert_eq!(restart_time("2027-01-15T08:10").unwrap(), Some(at(600)));
        assert_eq!(restart_time("2027-01-15T08:10:00").unwrap(), Some(at(600)));
        assert!(restart_time("tomorrow").is_err());
        assert_eq!(duration_text(59), "0 min");
        assert_eq!(duration_text(3_660), "1 h 1 min");
        assert_eq!(duration_text(90_000), "1 d 1 h");
    }

    #[test]
    fn parses_the_game_server_status() {
        let body = br#"{"medius_version":"0.12.0","gameplay_version":null,"started_at":"2026-11-26T10:00:00Z",
            "uptime_seconds":60,"mode":"registry","players_online":null,"queued":0,"queued_by_game_type":[0,0,0],
            "dme_ports":{"capacity":null,"in_use":0},"future_field":1,
            "worlds":[{"worker_id":"sabotage","state":"open","game_types":[0],"human_capacity":32,"pending":0,
              "match":{"profile_id":"p","game_type":0,"attacking_faction":0,"defending_faction":1,"humans":1,
                "attacking_humans":1,"defending_humans":0,"human_capacity":32,"bots":63,"world_capacity":64}}]}"#;
        let parsed: ServerStatus = serde_json::from_slice(body).unwrap();
        assert_eq!(parsed.worlds[0].current.as_ref().unwrap().bots, 63);
        assert_eq!(parsed.dme_ports.capacity, None);
        assert!(serde_json::from_slice::<ServerStatus>(br#"{"mode":"registry"}"#).is_err());
    }
}
