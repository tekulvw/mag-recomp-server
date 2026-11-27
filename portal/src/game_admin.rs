//! Client for the game server's (Medius) internal admin API, used by the admin panel's
//! Game server section.
//!
//! Every request carries a timestamp, a fresh nonce, the acting staff member and an
//! HMAC-SHA256 signature over
//!
//! ```text
//! METHOD "\n" PATH_AND_QUERY "\n" TIMESTAMP "\n" NONCE "\n" hex(SHA-256(body))
//! ```
//!
//! keyed with a shared secret (`PORTAL_GAME_ADMIN_SECRET_FILE`). A GET has an empty body, so
//! its body hash is the SHA-256 of zero bytes. Hex is lowercase. The secret never appears in
//! errors, logs or `Debug` output.

use std::fmt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use hmac::{Hmac, KeyInit, Mac};
use reqwest::{Method, StatusCode, Url};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

use crate::tokens;

pub const TIMESTAMP_HEADER: &str = "x-mag-admin-timestamp";
pub const NONCE_HEADER: &str = "x-mag-admin-nonce";
pub const ACTOR_HEADER: &str = "x-mag-admin-actor";
pub const SIGNATURE_HEADER: &str = "x-mag-admin-signature";

/// Shortest accepted shared secret, in bytes.
pub const MIN_SECRET_LEN: usize = 32;
/// How long a read, or a dry run that saves nothing, may take.
pub const READ_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a change that is applied may take. The game server can be slow to confirm a save that
/// it has made (a database waiting on its standby), so writes get longer than reads, and a write
/// that runs out of time is reported as unconfirmed, never as "nothing changed".
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(15);
/// Largest response body read; the character list is the biggest answer.
const MAX_RESPONSE_BYTES: usize = 4 << 20;
/// How long [`GameAdmin::character_keys`] reuses a character list it fetched.
pub const CHARACTER_LIST_TTL: Duration = Duration::from_secs(60);
/// How long it waits after a failed fetch before asking the game server again.
const CHARACTER_LIST_RETRY: Duration = Duration::from_secs(15);
/// The actor the game server records for the player report form's name lookups.
pub const PLAYER_LOOKUP_ACTOR: &str = "portal player reports (lookup)";

/// The last character list fetch: when, and the keys or the error.
type CachedKeys = (Instant, Result<Arc<Vec<String>>, String>);

/// Lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[usize::from(b >> 4)] as char);
        out.push(DIGITS[usize::from(b & 15)] as char);
    }
    out
}

/// The `X-Mag-Admin-Signature` value for one request.
pub fn signature(
    secret: &[u8],
    method: &str,
    path_and_query: &str,
    timestamp: u64,
    nonce: &str,
    body: &[u8],
) -> String {
    let message = format!(
        "{method}\n{path_and_query}\n{timestamp}\n{nonce}\n{}",
        hex(&Sha256::digest(body))
    );
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(message.as_bytes());
    hex(&mac.finalize().into_bytes())
}

/// Whether the game server sees the player signed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionState {
    Online,
    Offline,
    Unknown,
}

impl SessionState {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionState::Online => "online",
            SessionState::Offline => "offline",
            SessionState::Unknown => "unknown",
        }
    }
}

/// Highest character slot: an account holds up to three characters, in slots 0 to 2.
pub const MAX_SLOT: u8 = 2;

/// One entry of `GET /admin/v1/accounts`: one per character, deleted ones included. A game
/// server that predates character slots sends one entry per account without the character
/// fields, which are then `None` (and `deleted` false).
#[derive(Debug, Clone, Deserialize)]
pub struct CharacterSummary {
    pub account: String,
    pub level: u32,
    pub experience: i64,
    pub skill_points: i64,
    pub session: SessionState,
    #[serde(default)]
    pub character_id: Option<u32>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub slot: Option<u8>,
    #[serde(default)]
    pub faction: Option<u8>,
    /// Deleted by the player: kept by the game server, its slot free and its name reserved.
    #[serde(default)]
    pub deleted: bool,
    /// UTC `YYYY-MM-DDTHH:MM:SSZ`.
    #[serde(default)]
    pub deleted_at: Option<String>,
}

#[derive(Deserialize)]
struct AccountList {
    accounts: Vec<CharacterSummary>,
}

/// `GET /admin/v1/characters/{account}[?slot=N]`: the selected character (that slot, else the
/// account's lowest live slot) and every character of the account. From a game server that
/// predates character slots the character fields are `None` and `characters` is empty.
#[derive(Debug, Clone, Deserialize)]
pub struct Character {
    pub account: String,
    pub level: u32,
    pub experience: i64,
    pub skill_points: i64,
    pub next_level_experience: Option<i64>,
    pub session: SessionState,
    #[serde(default)]
    pub character_id: Option<u32>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub slot: Option<u8>,
    #[serde(default)]
    pub faction: Option<u8>,
    #[serde(default)]
    pub characters: Vec<AccountCharacter>,
}

/// One of an account's characters, live or deleted, in [`Character::characters`].
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct AccountCharacter {
    pub character_id: u32,
    pub name: String,
    pub slot: u8,
    pub faction: u8,
    pub level: u32,
    pub experience: i64,
    pub skill_points: i64,
    #[serde(default)]
    pub deleted: bool,
    /// UTC `YYYY-MM-DDTHH:MM:SSZ`.
    #[serde(default)]
    pub deleted_at: Option<String>,
}

/// A level change, planned (`applied: false`) or made.
#[derive(Debug, Clone, Deserialize)]
pub struct LevelChange {
    pub applied: bool,
    pub account: String,
    pub from_level: u32,
    pub to_level: u32,
    pub experience_before: i64,
    pub experience_after: i64,
    pub skill_points_before: i64,
    pub skill_points_after: i64,
    pub backup: Option<String>,
    /// The character changed; `None` from a game server that predates character slots.
    #[serde(default)]
    pub character_id: Option<u32>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub slot: Option<u8>,
}

/// A 409 answer: the game server will not make the change.
#[derive(Debug, Clone, Deserialize)]
pub struct Refusal {
    #[serde(rename = "error")]
    pub code: String,
    pub message: String,
}

impl Refusal {
    /// What staff can do about it.
    pub fn advice(&self) -> &'static str {
        match self.code.as_str() {
            "online" => {
                "The player is signed in to the game. Ask them to quit the game (or wait until \
                 they leave), then try again."
            }
            "session_unknown" => {
                "The game server cannot tell whether the player is signed in, so it will not \
                 change the character. Try again once the server reports their session."
            }
            "lower_level" => "Levels can only be raised. Choose a level above the current one.",
            "same_level" => "The character is already at that level. Nothing was changed.",
            "level_out_of_range" => "Choose a level the game supports.",
            "not_sanctioned" => {
                "The account has no active voice mute or ban of that kind, so there is nothing \
                 to lift. Nothing was changed."
            }
            "ambiguous_character" => {
                "The account has more than one character, so the game server will not guess \
                 which one to change. Open the account's character page, choose the character's \
                 slot and try again."
            }
            "slots_full" => {
                "Every character slot of the account is in use, so the deleted character cannot \
                 be restored until the player deletes one."
            }
            "no_progression_catalog" => {
                "The game server has no progression catalog loaded, so it cannot work out \
                 experience for a level. Fix the server's data files first."
            }
            _ => "Nothing was changed.",
        }
    }
}

/// One active voice sanction from `GET /admin/v1/voice`, or the `before`/`after` of a change.
/// Times are UTC `YYYY-MM-DDTHH:MM:SSZ`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct VoiceSanction {
    pub account: String,
    /// `mute` or `ban`.
    pub kind: String,
    pub reason: Option<String>,
    pub operator: String,
    pub created_at: String,
    /// `None` lasts until lifted.
    pub expires_at: Option<String>,
}

#[derive(Deserialize)]
struct VoiceList {
    sanctions: Vec<VoiceSanction>,
}

/// What a voice moderation request does to an account's in-match voice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceAction {
    Mute,
    Ban,
    Unmute,
    Unban,
}

impl VoiceAction {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "mute" => Some(Self::Mute),
            "ban" => Some(Self::Ban),
            "unmute" => Some(Self::Unmute),
            "unban" => Some(Self::Unban),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mute => "mute",
            Self::Ban => "ban",
            Self::Unmute => "unmute",
            Self::Unban => "unban",
        }
    }

    /// Unmute and unban lift a sanction; they take no duration.
    pub fn is_lift(self) -> bool {
        matches!(self, Self::Unmute | Self::Unban)
    }
}

/// Longest voice sanction with an end, in minutes (3650 days); longer ones are until lifted.
pub const MAX_VOICE_MINUTES: u32 = 3650 * 24 * 60;

/// A voice mute, ban or lift, planned (`applied: false`) or made.
#[derive(Debug, Clone, Deserialize)]
pub struct VoiceChange {
    pub applied: bool,
    pub account: String,
    pub action: String,
    pub kind: String,
    /// The active sanction of this kind before the change.
    pub before: Option<VoiceSanction>,
    /// The sanction after it; `None` for a lift.
    pub after: Option<VoiceSanction>,
}

/// `GET /admin/v1/status`: the game server's read-only status, counts only (no account names).
/// Fields the portal does not know are ignored, so the game server can add some.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ServerStatus {
    pub medius_version: Option<String>,
    pub gameplay_version: Option<String>,
    /// RFC 3339 time the Medius process started.
    pub started_at: Option<String>,
    pub uptime_seconds: u64,
    /// `registry`, `single` or `none`.
    pub mode: String,
    /// Accounts holding a live login lease; `None` when the game server cannot tell.
    pub players_online: Option<u32>,
    pub queued: u32,
    /// Humans waiting for a world, by requested game type.
    #[serde(default)]
    pub queued_by_game_type: Vec<u32>,
    pub dme_ports: DmePorts,
    /// Humans the whole server holds at once, across every world (the DME port cap); `None`
    /// when unbounded or from a game server that predates it.
    #[serde(default)]
    pub human_limit: Option<u32>,
    pub worlds: Vec<WorldStatus>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct DmePorts {
    /// `None` when unbounded.
    pub capacity: Option<u32>,
    pub in_use: u32,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct WorldStatus {
    pub worker_id: String,
    /// `open`, `draining`, `faulted` or `unavailable`.
    pub state: String,
    #[serde(default)]
    pub game_types: Vec<u8>,
    /// The configured row and profile cap on humans, before the server-wide DME port cap.
    pub human_capacity: u32,
    /// Humans the world can actually hold: `human_capacity` within the DME port cap. `None`
    /// from a game server that predates it.
    #[serde(default)]
    pub human_limit: Option<u32>,
    /// Characters in the world, humans and bots.
    #[serde(default)]
    pub world_size: Option<u32>,
    /// The population bots fill the world to, humans included; `None` without bot fill.
    #[serde(default)]
    pub bot_fill: Option<u32>,
    /// Humans placed on this world and not admitted yet.
    #[serde(default)]
    pub pending: u32,
    #[serde(rename = "match")]
    pub current: Option<MatchStatus>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct MatchStatus {
    pub profile_id: String,
    pub game_type: u8,
    pub attacking_faction: u8,
    pub defending_faction: u8,
    pub humans: u32,
    pub attacking_humans: u32,
    pub defending_humans: u32,
    pub human_capacity: u32,
    /// As [`WorldStatus::human_limit`], for the match.
    #[serde(default)]
    pub human_limit: Option<u32>,
    pub bots: u32,
    #[serde(default)]
    pub bot_fill: Option<u32>,
    pub world_capacity: u32,
}

#[derive(Deserialize)]
struct ErrorBody {
    error: String,
}

#[derive(Debug, thiserror::Error)]
pub enum GameAdminError {
    #[error(
        "The game server rejected the portal's admin credentials (HTTP 401). Check that the \
         portal and the game server use the same admin secret and that both clocks are right."
    )]
    Unauthorized,
    #[error("Could not reach the game server's admin API at {url}: {reason}.")]
    Unreachable { url: String, reason: String },
    /// A change was sent but not confirmed: it may or may not have been applied.
    #[error(
        "The request timed out; it may have been applied. The game server at {url} did not \
         confirm the change ({reason}). Refresh to check before trying again."
    )]
    Uncertain { url: String, reason: String },
    #[error("The game server's admin API gave an unexpected answer: {0}.")]
    Unexpected(String),
    #[error("The game server has no character for that account.")]
    NotFound,
    #[error("The game server rejected the request as invalid ({0}).")]
    BadRequest(String),
    #[error("The game server refused: {}", .0.message)]
    Refused(Refusal),
}

/// The configured admin API: base URL, shared secret and an HTTP client.
#[derive(Clone)]
pub struct GameAdmin {
    base_url: String,
    secret: Vec<u8>,
    http: reqwest::Client,
    /// The character keys, shared by every clone, for player name lookups.
    keys: Arc<Mutex<Option<CachedKeys>>>,
    read_timeout: Duration,
    write_timeout: Duration,
}

impl fmt::Debug for GameAdmin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GameAdmin")
            .field("base_url", &self.base_url)
            .field("secret", &"<redacted>")
            .finish()
    }
}

impl GameAdmin {
    /// `base_url` is an http or https origin (no path, query or credentials).
    pub fn new(base_url: &str, secret: Vec<u8>) -> anyhow::Result<Self> {
        let url = Url::parse(base_url.trim()).context("the game admin URL is not a valid URL")?;
        if !matches!(url.scheme(), "http" | "https") {
            bail!("the game admin URL must start with http:// or https://");
        }
        if url.host_str().is_none_or(str::is_empty)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            bail!("the game admin URL must be a bare origin such as http://host:port");
        }
        if secret.len() < MIN_SECRET_LEN {
            bail!("the game admin secret must be at least {MIN_SECRET_LEN} bytes");
        }
        // Each request sets its own overall timeout (reads and writes differ).
        let http = reqwest::Client::builder()
            .connect_timeout(READ_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            base_url: url.as_str().trim_end_matches('/').to_owned(),
            secret,
            http,
            keys: Arc::new(Mutex::new(None)),
            read_timeout: READ_TIMEOUT,
            write_timeout: WRITE_TIMEOUT,
        })
    }

    /// Other timeouts than [`READ_TIMEOUT`] and [`WRITE_TIMEOUT`] (synthetic tests).
    pub fn with_timeouts(mut self, read: Duration, write: Duration) -> Self {
        self.read_timeout = read;
        self.write_timeout = write;
        self
    }

    /// From the two settings, which must be given together. Empty values count as unset.
    pub fn from_config(
        url: Option<&str>,
        secret_file: Option<&Path>,
    ) -> anyhow::Result<Option<Self>> {
        let url = url.map(str::trim).filter(|u| !u.is_empty());
        let secret_file = secret_file.filter(|p| !p.as_os_str().is_empty());
        match (url, secret_file) {
            (None, None) => Ok(None),
            (Some(url), Some(file)) => {
                let raw = std::fs::read(file)
                    .with_context(|| format!("reading the game admin secret {}", file.display()))?;
                let secret = raw.trim_ascii().to_vec();
                Ok(Some(Self::new(url, secret)?))
            }
            _ => bail!(
                "set both PORTAL_GAME_ADMIN_URL and PORTAL_GAME_ADMIN_SECRET_FILE, or neither"
            ),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Every character the game server has.
    pub async fn accounts(&self, actor: &str) -> Result<Vec<CharacterSummary>, GameAdminError> {
        let (status, body) = self
            .send(Method::GET, "/admin/v1/accounts", Vec::new(), actor)
            .await?;
        match status {
            StatusCode::OK => Ok(parse::<AccountList>(&body, "the character list")?.accounts),
            other => Err(failure(other, &body)),
        }
    }

    /// Every character's key (lowercase name), sorted, from a list at most
    /// [`CHARACTER_LIST_TTL`] old. Player name lookups use it, so typing in the report form
    /// does not fetch the whole list on every key press. A failure is reused for a few seconds
    /// too, so an unreachable game server is not asked again at once.
    pub async fn character_keys(&self) -> Result<Arc<Vec<String>>, GameAdminError> {
        let now = Instant::now();
        let cached = self.keys.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some((at, result)) = cached {
            let age = now.duration_since(at);
            match result {
                Ok(keys) if age < CHARACTER_LIST_TTL => return Ok(keys),
                Err(reason) if age < CHARACTER_LIST_RETRY => {
                    return Err(GameAdminError::Unexpected(reason));
                }
                _ => {}
            }
        }
        let fetched = self.accounts(PLAYER_LOOKUP_ACTOR).await.map(|list| {
            let mut keys: Vec<String> = list
                .into_iter()
                .map(|c| c.account.to_ascii_lowercase())
                .filter(|k| valid_account(k))
                .collect();
            keys.sort();
            keys.dedup();
            Arc::new(keys)
        });
        let stored = fetched.as_ref().map(Arc::clone).map_err(|e| e.to_string());
        *self.keys.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), stored));
        fetched
    }

    /// The game server's status. `NotFound` means it is too old to report one.
    pub async fn status(&self, actor: &str) -> Result<ServerStatus, GameAdminError> {
        self.status_logged(actor, true).await
    }

    /// As [`GameAdmin::status`], without a warning per failure: the status poller logs only
    /// when a game server goes down or comes back.
    pub async fn status_quietly(&self, actor: &str) -> Result<ServerStatus, GameAdminError> {
        self.status_logged(actor, false).await
    }

    async fn status_logged(&self, actor: &str, warn: bool) -> Result<ServerStatus, GameAdminError> {
        let (status, body) = self
            .send_logged(
                Method::GET,
                "/admin/v1/status",
                Vec::new(),
                actor,
                warn,
                false,
            )
            .await?;
        match status {
            StatusCode::OK => parse(&body, "the server status"),
            other => Err(failure(other, &body)),
        }
    }

    /// One character, or `None` if the game server has no live one for `account` (in `slot`,
    /// when given; else the account's lowest live slot).
    pub async fn character(
        &self,
        actor: &str,
        account: &str,
        slot: Option<u8>,
    ) -> Result<Option<Character>, GameAdminError> {
        let mut path = format!("/admin/v1/characters/{}", path_segment(account)?);
        if let Some(slot) = slot {
            path.push_str(&format!("?slot={}", slot_number(slot)?));
        }
        let (status, body) = self.send(Method::GET, &path, Vec::new(), actor).await?;
        match status {
            StatusCode::OK => {
                let character: Character = parse(&body, "the character")?;
                if character.account != account {
                    return Err(GameAdminError::Unexpected(
                        "it described a different account".into(),
                    ));
                }
                // A game server that predates slots ignores the query and has only slot 0.
                if let Some(asked) = slot
                    && character.slot.unwrap_or(0) != asked
                {
                    return Err(GameAdminError::Unexpected(
                        "it described a different character slot".into(),
                    ));
                }
                Ok(Some(character))
            }
            StatusCode::NOT_FOUND => Ok(None),
            other => Err(failure(other, &body)),
        }
    }

    /// Plans (`apply: false`) or makes a level change, to the character in `slot`. Without a
    /// slot the game server changes the account's only live character, and refuses with
    /// `ambiguous_character` when it has more than one.
    pub async fn set_level(
        &self,
        actor: &str,
        account: &str,
        level: u32,
        apply: bool,
        slot: Option<u8>,
    ) -> Result<LevelChange, GameAdminError> {
        #[derive(serde::Serialize)]
        struct Body {
            level: u32,
            apply: bool,
            #[serde(skip_serializing_if = "Option::is_none")]
            slot: Option<u8>,
        }
        let path = format!("/admin/v1/characters/{}/level", path_segment(account)?);
        if let Some(slot) = slot {
            slot_number(slot)?;
        }
        let body = serde_json::to_vec(&Body { level, apply, slot }).expect("plain JSON");
        let (status, body) = self
            .send_logged(Method::POST, &path, body, actor, true, apply)
            .await?;
        match status {
            StatusCode::OK => {
                let change: LevelChange = parse(&body, "the level change")?;
                // An answer without a slot comes from a game server that predates slots.
                let other_slot = slot.is_some_and(|s| change.slot.is_some_and(|c| c != s));
                if change.account != account
                    || change.to_level != level
                    || change.applied != apply
                    || other_slot
                {
                    return Err(GameAdminError::Unexpected(
                        "it described a different change than the one asked for".into(),
                    ));
                }
                Ok(change)
            }
            other => Err(failure(other, &body)),
        }
    }

    /// The active voice mutes and bans. `NotFound` means the game server is too old to have them.
    pub async fn voice_sanctions(&self, actor: &str) -> Result<Vec<VoiceSanction>, GameAdminError> {
        let (status, body) = self
            .send(Method::GET, "/admin/v1/voice", Vec::new(), actor)
            .await?;
        match status {
            StatusCode::OK => Ok(parse::<VoiceList>(&body, "the voice sanctions")?.sanctions),
            other => Err(failure(other, &body)),
        }
    }

    /// Plans (`apply: false`) or makes a voice mute, ban or lift. `duration_minutes` is only
    /// for a mute or ban; `None` lasts until lifted.
    pub async fn change_voice(
        &self,
        actor: &str,
        account: &str,
        action: VoiceAction,
        apply: bool,
        reason: Option<&str>,
        duration_minutes: Option<u32>,
    ) -> Result<VoiceChange, GameAdminError> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            apply: bool,
            #[serde(skip_serializing_if = "Option::is_none")]
            reason: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            duration_minutes: Option<u32>,
        }
        if action.is_lift() && duration_minutes.is_some() {
            return Err(GameAdminError::BadRequest(
                "a lift takes no duration".into(),
            ));
        }
        let path = format!(
            "/admin/v1/voice/{}/{}",
            path_segment(account)?,
            action.as_str()
        );
        let body = serde_json::to_vec(&Body {
            apply,
            reason,
            duration_minutes,
        })
        .expect("plain JSON");
        let (status, body) = self
            .send_logged(Method::POST, &path, body, actor, true, apply)
            .await?;
        match status {
            StatusCode::OK => {
                let change: VoiceChange = parse(&body, "the voice change")?;
                if change.account != account
                    || change.action != action.as_str()
                    || change.applied != apply
                {
                    return Err(GameAdminError::Unexpected(
                        "it described a different change than the one asked for".into(),
                    ));
                }
                Ok(change)
            }
            other => Err(failure(other, &body)),
        }
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        body: Vec<u8>,
        actor: &str,
    ) -> Result<(StatusCode, Vec<u8>), GameAdminError> {
        self.send_logged(method, path, body, actor, true, false)
            .await
    }

    /// `applies` marks a request that changes something: it gets [`WRITE_TIMEOUT`], and once it
    /// may have reached the game server, a timeout or a dropped answer is
    /// [`GameAdminError::Uncertain`] rather than [`GameAdminError::Unreachable`].
    async fn send_logged(
        &self,
        method: Method,
        path: &str,
        body: Vec<u8>,
        actor: &str,
        warn: bool,
        applies: bool,
    ) -> Result<(StatusCode, Vec<u8>), GameAdminError> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let timeout = if applies {
            self.write_timeout
        } else {
            self.read_timeout
        };
        let nonce = tokens::b64url(&tokens::random_bytes::<18>());
        let signature = signature(
            &self.secret,
            method.as_str(),
            path,
            timestamp,
            &nonce,
            &body,
        );
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.base_url))
            .header(TIMESTAMP_HEADER, timestamp.to_string())
            .header(NONCE_HEADER, nonce)
            .header(ACTOR_HEADER, actor)
            .header(SIGNATURE_HEADER, signature)
            .timeout(timeout);
        if !body.is_empty() {
            request = request
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body);
        }
        let unreachable = |e: reqwest::Error| {
            let reason = if e.is_timeout() {
                format!("no answer within {}", seconds(timeout))
            } else if e.is_connect() {
                "the connection failed".to_owned()
            } else {
                "the request failed".to_owned()
            };
            // A change that may have reached the game server is not "unreachable": it may be saved.
            if applies && !e.is_connect() {
                if warn {
                    tracing::warn!(url = %self.base_url, reason, "game admin API did not confirm a change");
                }
                return GameAdminError::Uncertain {
                    url: self.base_url.clone(),
                    reason,
                };
            }
            if warn {
                tracing::warn!(url = %self.base_url, reason, "game admin API unreachable");
            }
            GameAdminError::Unreachable {
                url: self.base_url.clone(),
                reason,
            }
        };
        let response = request.send().await.map_err(unreachable)?;
        let status = response.status();
        if response
            .content_length()
            .is_some_and(|n| n > MAX_RESPONSE_BYTES as u64)
        {
            return Err(GameAdminError::Unexpected(
                "the answer was too large".into(),
            ));
        }
        let bytes = response.bytes().await.map_err(unreachable)?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err(GameAdminError::Unexpected(
                "the answer was too large".into(),
            ));
        }
        if status == StatusCode::UNAUTHORIZED {
            if warn {
                tracing::warn!(url = %self.base_url, "game admin API rejected the portal's signature");
            }
            return Err(GameAdminError::Unauthorized);
        }
        // The game server's own "the database stopped answering while saving" (HTTP 503).
        if applies && status == StatusCode::SERVICE_UNAVAILABLE && outcome_unknown(&bytes) {
            if warn {
                tracing::warn!(url = %self.base_url, "game admin API could not confirm a change");
            }
            return Err(GameAdminError::Uncertain {
                url: self.base_url.clone(),
                reason: "its database stopped answering while saving".into(),
            });
        }
        Ok((status, bytes.to_vec()))
    }
}

/// "5 seconds", "1 second" or "500 ms".
fn seconds(d: Duration) -> String {
    match d.as_secs() {
        0 => format!("{} ms", d.as_millis()),
        1 => "1 second".into(),
        n => format!("{n} seconds"),
    }
}

/// The game server's 503 body for a save whose outcome it does not know.
fn outcome_unknown(body: &[u8]) -> bool {
    serde_json::from_slice::<ErrorBody>(body).is_ok_and(|e| e.error == "outcome_unknown")
}

/// The account as a URL path segment: lowercase letters, digits, `-` and `_` need no escaping.
fn path_segment(account: &str) -> Result<&str, GameAdminError> {
    if valid_account(account) {
        Ok(account)
    } else {
        Err(GameAdminError::BadRequest(
            "not a valid account name".into(),
        ))
    }
}

/// A character slot the game server can hold, or `BadRequest`.
fn slot_number(slot: u8) -> Result<u8, GameAdminError> {
    if slot <= MAX_SLOT {
        Ok(slot)
    } else {
        Err(GameAdminError::BadRequest("not a character slot".into()))
    }
}

/// A game server account key: a lowercase player name of 1 to 32 letters, digits, `-` or `_`.
pub fn valid_account(account: &str) -> bool {
    (1..=32).contains(&account.len())
        && account
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

fn parse<T: DeserializeOwned>(body: &[u8], what: &str) -> Result<T, GameAdminError> {
    serde_json::from_slice(body)
        .map_err(|_| GameAdminError::Unexpected(format!("{what} was not in the expected form")))
}

/// The error for a status the endpoint does not succeed with.
fn failure(status: StatusCode, body: &[u8]) -> GameAdminError {
    match status {
        StatusCode::NOT_FOUND => GameAdminError::NotFound,
        StatusCode::CONFLICT => match serde_json::from_slice::<Refusal>(body) {
            Ok(refusal) => GameAdminError::Refused(refusal),
            Err(_) => GameAdminError::Unexpected("HTTP 409 without a reason".into()),
        },
        StatusCode::BAD_REQUEST => GameAdminError::BadRequest(
            serde_json::from_slice::<ErrorBody>(body)
                .map(|e| e.error)
                .unwrap_or_else(|_| "no reason given".into()),
        ),
        other => GameAdminError::Unexpected(format!("HTTP {}", other.as_u16())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"0123456789abcdef0123456789abcdef";

    /// Fixed vectors for other implementations (the Medius admin API). Computed independently
    /// with Python's `hmac` and `hashlib`:
    /// `hmac.new(secret, msg, sha256).hexdigest()` where
    /// `msg = f"{method}\n{path}\n{ts}\n{nonce}\n{sha256(body).hexdigest()}"`.
    #[test]
    fn signature_vectors() {
        assert_eq!(
            hex(&Sha256::digest(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            signature(
                SECRET,
                "POST",
                "/admin/v1/characters/alpha/level",
                1_800_000_000,
                "test-nonce-0123456789",
                br#"{"level":20,"apply":false}"#,
            ),
            "67830476b109612f9f91f3c0b40f996288f6324dafe3e5d173d7cf1152727e9c"
        );
        assert_eq!(
            signature(
                SECRET,
                "GET",
                "/admin/v1/accounts",
                1_800_000_000,
                "test-nonce-0123456789",
                b"",
            ),
            "75f73e3794341809c53bc58ac4a3a854e390514e6473d24aaf845a81f445e453"
        );
    }

    #[test]
    fn hex_is_lowercase() {
        assert_eq!(hex(&[0x00, 0x0f, 0xa0, 0xff]), "000fa0ff");
    }

    #[test]
    fn configuration() {
        let ok = |url: &str| GameAdmin::new(url, SECRET.to_vec());
        assert_eq!(
            ok("http://mag-medius-admin.mag.svc:10490/")
                .unwrap()
                .base_url(),
            "http://mag-medius-admin.mag.svc:10490"
        );
        assert!(ok("https://admin.example.test").is_ok());
        for bad in [
            "ftp://host",
            "file:///etc/passwd",
            "host:10490",
            "http://host/prefix",
            "http://host/?q=1",
            "http://user:pw@host",
            "http://host/#x",
        ] {
            assert!(ok(bad).is_err(), "{bad}");
        }
        let short = GameAdmin::new("http://host", b"short".to_vec()).unwrap_err();
        assert!(short.to_string().contains("at least 32 bytes"));
        let debug = format!("{:?}", ok("http://host").unwrap());
        assert!(debug.contains("redacted") && !debug.contains("0123456789abcdef"));

        // Both settings or neither.
        assert!(GameAdmin::from_config(None, None).unwrap().is_none());
        assert!(GameAdmin::from_config(Some(""), None).unwrap().is_none());
        let dir = std::env::temp_dir().join(format!(
            "portal-game-admin-{}",
            tokens::b64url(&tokens::random_bytes::<9>())
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("secret");
        std::fs::write(&file, [SECRET, b"\n"].concat()).unwrap();
        assert!(GameAdmin::from_config(Some("http://host"), None).is_err());
        assert!(GameAdmin::from_config(None, Some(&file)).is_err());
        let both = GameAdmin::from_config(Some("http://host"), Some(&file))
            .unwrap()
            .unwrap();
        assert_eq!(both.secret, SECRET);
        std::fs::write(&file, "too short\n").unwrap();
        let err = GameAdmin::from_config(Some("http://host"), Some(&file)).unwrap_err();
        assert!(!format!("{err:#}").contains("too short"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn errors_never_show_the_secret() {
        let refusal = failure(
            StatusCode::CONFLICT,
            br#"{"error":"online","message":"alpha is online"}"#,
        );
        match &refusal {
            GameAdminError::Refused(r) => {
                assert_eq!(r.code, "online");
                assert!(r.advice().contains("quit"));
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            failure(StatusCode::CONFLICT, b"nope"),
            GameAdminError::Unexpected(_)
        ));
        assert!(matches!(
            failure(StatusCode::BAD_REQUEST, br#"{"error":"invalid_request"}"#),
            GameAdminError::BadRequest(e) if e == "invalid_request"
        ));
        assert!(matches!(
            failure(StatusCode::INTERNAL_SERVER_ERROR, b""),
            GameAdminError::Unexpected(e) if e == "HTTP 500"
        ));
        for e in [
            GameAdminError::Unauthorized,
            GameAdminError::Unreachable {
                url: "http://host".into(),
                reason: "the connection failed".into(),
            },
            GameAdminError::Uncertain {
                url: "http://host".into(),
                reason: "no answer within 15 seconds".into(),
            },
            refusal,
        ] {
            assert!(!e.to_string().contains("0123456789abcdef"));
        }
    }

    #[test]
    fn write_outcomes() {
        assert_eq!(seconds(Duration::from_secs(5)), "5 seconds");
        assert_eq!(seconds(Duration::from_secs(1)), "1 second");
        assert_eq!(seconds(Duration::from_millis(300)), "300 ms");
        assert!(outcome_unknown(
            br#"{"error":"outcome_unknown","message":"x"}"#
        ));
        assert!(!outcome_unknown(br#"{"error":"internal_error"}"#));
        assert!(!outcome_unknown(b"not json"));
        let text = GameAdminError::Uncertain {
            url: "http://host".into(),
            reason: "no answer within 15 seconds".into(),
        }
        .to_string();
        assert!(text.contains("timed out; it may have been applied") && text.contains("Refresh"));
        assert!(!text.contains("Could not reach"));
    }

    #[test]
    fn strict_parsing() {
        let summary: AccountList = serde_json::from_slice(
            br#"{"accounts":[{"account":"alpha","level":5,"experience":100,"skill_points":2,"session":"online"}]}"#,
        )
        .unwrap();
        assert_eq!(summary.accounts[0].session, SessionState::Online);
        assert!(
            parse::<AccountList>(
                br#"{"accounts":[{"account":"alpha","level":5,"experience":100,"skill_points":2,"session":"asleep"}]}"#,
                "x"
            )
            .is_err()
        );
        assert!(
            parse::<Character>(
                br#"{"account":"alpha","level":"5","experience":1,"skill_points":2,"next_level_experience":null,"session":"offline"}"#,
                "x"
            )
            .is_err()
        );
        assert!(
            parse::<AccountList>(
                br#"{"accounts":[{"account":"alpha","level":5,"experience":100,"skill_points":2,"session":"online","slot":"one"}]}"#,
                "x"
            )
            .is_err()
        );
        assert!(valid_account("alpha_ranger-7"));
        for bad in ["", "Alpha", "a/b", "a b", "a%2f", &"a".repeat(33)] {
            assert!(!valid_account(bad), "{bad}");
        }
    }

    #[test]
    fn account_lists_old_and_new() {
        // A game server that predates character slots: one entry per account.
        let old: AccountList = serde_json::from_slice(
            br#"{"accounts":[{"account":"alpha","level":5,"experience":100,"skill_points":2,"session":"offline"}]}"#,
        )
        .unwrap();
        let a = &old.accounts[0];
        assert_eq!((a.account.as_str(), a.level), ("alpha", 5));
        assert_eq!((a.character_id, a.slot, a.faction), (None, None, None));
        assert!(a.name.is_none() && !a.deleted && a.deleted_at.is_none());

        // One entry per character, a deleted one included.
        let new: AccountList = serde_json::from_slice(
            br#"{"accounts":[
              {"account":"alpha","level":5,"experience":100,"skill_points":2,"session":"offline",
               "character_id":11,"name":"Alpha","slot":0,"faction":1,"deleted":false,"deleted_at":null},
              {"account":"alpha","level":30,"experience":9000,"skill_points":40,"session":"offline",
               "character_id":12,"name":"AlphaTwo","slot":2,"faction":0,"deleted":false,"deleted_at":null},
              {"account":"alpha","level":2,"experience":10,"skill_points":0,"session":"offline",
               "character_id":9,"name":"AlphaOld","slot":1,"faction":2,"deleted":true,
               "deleted_at":"2026-11-20T08:30:00Z"}]}"#,
        )
        .unwrap();
        assert_eq!(new.accounts.len(), 3);
        let second = &new.accounts[1];
        assert_eq!(second.character_id, Some(12));
        assert_eq!(second.name.as_deref(), Some("AlphaTwo"));
        assert_eq!((second.slot, second.faction), (Some(2), Some(0)));
        let gone = &new.accounts[2];
        assert!(gone.deleted);
        assert_eq!(gone.deleted_at.as_deref(), Some("2026-11-20T08:30:00Z"));
    }

    #[test]
    fn characters_of_an_account() {
        let old: Character = serde_json::from_slice(
            br#"{"account":"alpha","level":5,"experience":1,"skill_points":2,"next_level_experience":null,"session":"offline"}"#,
        )
        .unwrap();
        assert!(old.characters.is_empty() && old.slot.is_none() && old.name.is_none());

        let new: Character = serde_json::from_slice(
            br#"{"account":"alpha","level":30,"experience":9000,"skill_points":40,
                 "next_level_experience":9500,"session":"offline","character_id":12,
                 "name":"AlphaTwo","slot":2,"faction":0,"characters":[
                   {"character_id":11,"name":"Alpha","slot":0,"faction":1,"level":5,
                    "experience":100,"skill_points":2,"deleted":false,"deleted_at":null},
                   {"character_id":12,"name":"AlphaTwo","slot":2,"faction":0,"level":30,
                    "experience":9000,"skill_points":40,"deleted":false,"deleted_at":null},
                   {"character_id":9,"name":"AlphaOld","slot":1,"faction":2,"level":2,
                    "experience":10,"skill_points":0,"deleted":true,
                    "deleted_at":"2026-11-20T08:30:00Z"}]}"#,
        )
        .unwrap();
        assert_eq!((new.slot, new.name.as_deref()), (Some(2), Some("AlphaTwo")));
        assert_eq!(new.characters.len(), 3);
        assert!(new.characters[2].deleted);
        assert_eq!(new.characters[0].faction, 1);
    }

    #[test]
    fn slot_refusals_have_advice() {
        for (code, words) in [
            ("ambiguous_character", "choose the character"),
            ("slots_full", "cannot be restored"),
        ] {
            let refusal = failure(
                StatusCode::CONFLICT,
                format!(r#"{{"error":"{code}","message":"synthetic"}}"#).as_bytes(),
            );
            match refusal {
                GameAdminError::Refused(r) => {
                    assert_eq!(r.code, code);
                    assert!(r.advice().contains(words), "{code}: {}", r.advice());
                }
                other => panic!("{other:?}"),
            }
        }
    }

    /// The path, query and body of each request, answered with the JSON `answer` gives for the
    /// path and body.
    type Seen = Arc<Mutex<Vec<(String, String)>>>;

    async fn capture(
        answer: fn(&str, &serde_json::Value) -> serde_json::Value,
    ) -> (Seen, GameAdmin) {
        use axum::response::IntoResponse;
        let seen: Seen = Arc::default();
        let log = seen.clone();
        let app =
            axum::Router::new().fallback(move |uri: axum::http::Uri, body: axum::body::Bytes| {
                let log = log.clone();
                async move {
                    let path = uri.path_and_query().unwrap().as_str().to_owned();
                    let text = String::from_utf8(body.to_vec()).unwrap();
                    log.lock().unwrap().push((path.clone(), text.clone()));
                    let request = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
                    axum::Json(answer(&path, &request)).into_response()
                }
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let api = GameAdmin::new(&format!("http://{addr}"), SECRET.to_vec()).unwrap();
        (seen, api)
    }

    fn character_answer(path: &str, _: &serde_json::Value) -> serde_json::Value {
        let slot: u8 = path
            .split_once("?slot=")
            .map_or(0, |(_, n)| n.parse().unwrap());
        serde_json::json!({"account": "alpha", "level": 5, "experience": 1, "skill_points": 2,
            "next_level_experience": null, "session": "offline", "character_id": 10 + slot,
            "name": format!("Alpha{slot}"), "slot": slot, "faction": 0, "characters": []})
    }

    fn level_answer(_: &str, request: &serde_json::Value) -> serde_json::Value {
        let mut answer = serde_json::json!({"applied": request["apply"], "account": "alpha",
            "from_level": 5, "to_level": request["level"], "experience_before": 1,
            "experience_after": 2, "skill_points_before": 3, "skill_points_after": 4,
            "backup": null});
        // Like a game server that predates slots when none was asked for.
        if let Some(slot) = request.get("slot") {
            answer["slot"] = slot.clone();
            answer["name"] = "AlphaTwo".into();
            answer["character_id"] = 12.into();
        }
        answer
    }

    #[tokio::test]
    async fn slot_query_and_body() {
        let (seen, api) = capture(character_answer).await;
        let c = api
            .character("staff", "alpha", None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(c.slot, Some(0));
        let c = api
            .character("staff", "alpha", Some(2))
            .await
            .unwrap()
            .unwrap();
        assert_eq!((c.slot, c.name.as_deref()), (Some(2), Some("Alpha2")));
        assert!(matches!(
            api.character("staff", "alpha", Some(3)).await,
            Err(GameAdminError::BadRequest(_))
        ));
        let paths: Vec<String> = seen
            .lock()
            .unwrap()
            .iter()
            .map(|(p, _)| p.clone())
            .collect();
        assert_eq!(
            paths,
            [
                "/admin/v1/characters/alpha",
                "/admin/v1/characters/alpha?slot=2"
            ]
        );

        let (seen, api) = capture(level_answer).await;
        let change = api
            .set_level("staff", "alpha", 20, false, None)
            .await
            .unwrap();
        assert_eq!((change.slot, change.name), (None, None));
        let change = api
            .set_level("staff", "alpha", 20, true, Some(2))
            .await
            .unwrap();
        assert_eq!(change.slot, Some(2));
        assert_eq!(change.name.as_deref(), Some("AlphaTwo"));
        let bodies: Vec<String> = seen
            .lock()
            .unwrap()
            .iter()
            .map(|(_, b)| b.clone())
            .collect();
        assert_eq!(
            bodies,
            [
                r#"{"level":20,"apply":false}"#,
                r#"{"level":20,"apply":true,"slot":2}"#
            ]
        );
    }

    #[tokio::test]
    async fn answers_about_another_slot_are_rejected() {
        fn wrong_slot(path: &str, request: &serde_json::Value) -> serde_json::Value {
            let mut answer = if path.ends_with("/level") {
                level_answer(path, request)
            } else {
                character_answer("", request)
            };
            answer["slot"] = 1.into();
            answer
        }
        let (_, api) = capture(wrong_slot).await;
        assert!(matches!(
            api.character("staff", "alpha", Some(2)).await,
            Err(GameAdminError::Unexpected(_))
        ));
        assert!(matches!(
            api.set_level("staff", "alpha", 20, false, Some(2)).await,
            Err(GameAdminError::Unexpected(_))
        ));

        // A game server that predates slots answers without one: only slot 0 can be it.
        fn no_slot(_: &str, _: &serde_json::Value) -> serde_json::Value {
            serde_json::json!({"account": "alpha", "level": 5, "experience": 1,
                "skill_points": 2, "next_level_experience": null, "session": "offline"})
        }
        let (_, api) = capture(no_slot).await;
        assert!(
            api.character("staff", "alpha", Some(0))
                .await
                .unwrap()
                .is_some()
        );
        assert!(matches!(
            api.character("staff", "alpha", Some(1)).await,
            Err(GameAdminError::Unexpected(_))
        ));
    }
}
