//! Local admission and evidence sink. SCERT authentication remains in Horizon.
pub mod active_abilities;
mod actor_collision;
mod admission;
pub mod armor;
pub mod ballistics;
pub mod blast_rules;
pub mod blast_visibility;
pub mod bootstrap_loadouts;
pub mod bot_firearms;
pub mod bot_healing;
pub mod bot_names;
pub mod bot_navigation;
mod bot_objectives;
pub mod bot_physics;
pub mod bot_registry;
mod bot_traffic_costs;
mod bot_traffic_flow;
pub mod bullet_hit;
pub mod collision;
pub mod damage;
mod death_notifications;
mod development_bot_movement;
pub mod leadership_passives;
pub mod leadership_runtime;
pub mod navmesh;
pub mod retail_movement;
pub use development_bot_movement::{BotMove, BotMoveMode};
mod development_target;
pub use development_target::BotScript;
pub mod dummy;
mod event_decode;
mod events;
pub mod fall_damage;
use event_decode::DecodedEvent;
mod resources;
mod world_tick;
pub use resources::EngineResources;
pub mod healing;
pub mod item_catalog;
pub mod loadouts;
mod match_flow;
mod multiplayer;
pub mod objective_catalog;
mod objective_control;
pub mod owner_update;
pub mod player_tracking;
pub mod progression;
pub mod projectile;
pub mod projectile_replication;
pub mod protocol;
pub mod replication;
pub mod retail_armory;
pub mod retail_combat;
pub mod retail_melee;
mod score_clock;
pub mod skill_catalog;
pub mod spawn_catalog;
mod spawn_progression;
pub mod structure_catalog;
pub mod structures;
pub mod telemetry;
pub mod turret;
pub mod vehicle_exit;
pub mod wire_profile;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

pub const BRIDGE_VERSION: u8 = 2;
pub const MAX_LINE: usize = 65_536;
pub const MAX_PAYLOAD: usize = 16_384;
pub const MAX_LOG: u64 = 64 * 1024 * 1024;
pub const IO_DEADLINE: Duration = Duration::from_secs(2);
pub const ACTIVE_MAP_NAME: &str = "mag_unalaska_final";
pub(crate) const MAX_MATCH_PLAYERS: usize = 64;
const IDLE_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Character {
    pub name: String,
    pub faction: u8,
    pub head: u8,
    pub voice: u8,
    pub universe_id: u32,
    #[serde(default)]
    pub character_id: Option<u32>,
    #[serde(default)]
    pub loadouts: Option<Box<loadouts::SavedLoadouts>>,
    #[serde(default, deserialize_with = "deserialize_skills")]
    pub skills: Option<[u32; 5]>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub progression: Option<progression::ProgressionSnapshot>,
    #[serde(default)]
    pub leadership: Option<leadership_runtime::LeadershipRecord>,
}
fn deserialize_skills<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<[u32; 5]>, D::Error> {
    <[u32; 5]>::deserialize(deserializer).map(Some)
}
impl Active {
    /// Character field18. Nominations and leadership level follow native
    /// Initialize/SetNominations; specialty remains the existing fixed value.
    fn mag_info(&self, level: u8) -> replication::MagCharacterInfo {
        let c = &self.reservation.character;
        replication::MagCharacterInfo {
            nominations: self.nominations,
            level,
            leadership_level: self.leadership_level,
            skill_specialty: 4,
            head_id: c.head,
            voice_id: c.voice,
        }
    }
}
impl Character {
    fn loadout(
        &self,
        catalog: &item_catalog::ItemCatalog,
        index: u8,
        profile: wire_profile::WireProfile,
    ) -> Option<replication::CharacterLoadout> {
        // Legacy authority resolver. Retail combat uses its separate catalog;
        // saved retail replication uses the disk-backed Armory below.
        if !profile.catalog_item_ids_valid() {
            return None;
        }
        match &self.loadouts {
            Some(saved) => saved.loadout(catalog, index),
            None => catalog.valor_loadout(index),
        }
    }
    /// Equipment replication has a separately verified, optional retail source.
    /// The authority resolver above deliberately remains prototype-only.
    fn replication_loadout(
        &self,
        catalog: &item_catalog::ItemCatalog,
        index: u8,
        profile: wire_profile::WireProfile,
    ) -> Option<replication::CharacterLoadout> {
        if profile.application_id() == 20231 {
            if let (Some(saved), Some(armory)) = (&self.loadouts, catalog.retail_armory()) {
                catalog.validate_saved_item_bindings(saved).ok()?;
                return armory.saved_loadout(self.faction, saved, index);
            }
            return catalog.bootstrap_loadout(self.faction, index);
        }
        self.loadout(catalog, index, profile)
    }
    fn replication_default_inventory_index(
        &self,
        catalog: &item_catalog::ItemCatalog,
        profile: wire_profile::WireProfile,
    ) -> Option<u8> {
        if profile.application_id() == 20231 {
            if catalog.retail_armory().is_some()
                && let Some(saved) = &self.loadouts
            {
                return Some(saved.selected_index);
            }
            if let Some(index) = catalog.bootstrap_default_inventory_index(self.faction) {
                return Some(index);
            }
        }
        self.loadouts.as_ref().map(|saved| saved.selected_index)
    }
    fn initial_inventory_index(&self) -> u8 {
        self.loadouts
            .as_ref()
            .map_or(0, |saved| saved.selected_index)
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    Tcp,
    Udp,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    ClientToServer,
}

// Explicit variants avoid serde flatten's interaction with unknown-field checks.
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Hello {
        v: u8,
        request_id: String,
    },
    ConfigureMatch {
        v: u8,
        request_id: String,
        minimum_players: usize,
        #[serde(default)]
        minimum_players_per_team: [usize; 2],
        maximum_players: usize,
        #[serde(default = "wire_profile::default_application_id")]
        application_id: u32,
    },
    Reserve {
        v: u8,
        request_id: String,
        reservation_id: String,
        world_id: u32,
        player_id: u16,
        account_id: u32,
        session_key: String,
        access_key: String,
        character: Character,
    },
    Event {
        v: u8,
        request_id: String,
        reservation_id: String,
        world_id: u32,
        player_id: u16,
        transport: Transport,
        direction: Direction,
        rt_type: u8,
        plugin_id: Option<u8>,
        payload_hex: String,
    },
    ProgressionCommitted {
        v: u8,
        request_id: String,
        reservation_id: String,
        world_id: u32,
        player_id: u16,
        reward_id: String,
        progression: progression::ProgressionSnapshot,
        match_experience: u32,
    },
    DevelopmentSpawnTarget {
        v: u8,
        request_id: String,
        reservation_id: String,
        world_id: u32,
        player_id: u16,
        spawn_id: String,
        expected_generation: u32,
        position: replication::CharacterPosition,
        #[serde(default)]
        bot_script: Option<BotScript>,
    },
    DevelopmentMoveBot {
        v: u8,
        request_id: String,
        reservation_id: String,
        world_id: u32,
        player_id: u16,
        movement_id: String,
        expected_owner_generation: u32,
        expected_bot_generation: u32,
        movement: BotMove,
    },
    DevelopmentDamage {
        v: u8,
        request_id: String,
        reservation_id: String,
        world_id: u32,
        player_id: u16,
        damage_id: String,
        target_character_id: u32,
        expected_generation: u32,
        damage: damage::ResolvedDamage,
    },
    DevelopmentArmDemolition {
        v: u8,
        request_id: String,
        reservation_id: String,
        world_id: u32,
        player_id: u16,
        object_index: u16,
    },
    DevelopmentStructureDamage {
        v: u8,
        request_id: String,
        reservation_id: String,
        world_id: u32,
        player_id: u16,
        damage_id: String,
        target_object_id: u32,
        amount_hp: f32,
    },
    Release {
        v: u8,
        request_id: String,
        reservation_id: String,
    },
}
impl Request {
    /// Classification through the native typed codec, without exporting packet
    /// bytes or identity material into performance telemetry.
    pub fn is_gameplay_update(&self) -> bool {
        if let Self::Event {
            rt_type: 0x3d,
            plugin_id: Some(1),
            payload_hex,
            ..
        } = self
        {
            if validate_payload_hex(payload_hex).is_err() {
                return false;
            }
            // Telemetry classifies only the application tag. Full native decode
            // belongs to Engine with its idle-selected wire profile.
            payload_hex.get(..4) == Some("0007")
        } else {
            false
        }
    }
    fn header(&self) -> (u8, &str) {
        match self {
            Self::Hello { v, request_id }
            | Self::ConfigureMatch { v, request_id, .. }
            | Self::Reserve { v, request_id, .. }
            | Self::Event { v, request_id, .. }
            | Self::ProgressionCommitted { v, request_id, .. }
            | Self::DevelopmentSpawnTarget { v, request_id, .. }
            | Self::DevelopmentMoveBot { v, request_id, .. }
            | Self::DevelopmentDamage { v, request_id, .. }
            | Self::DevelopmentArmDemolition { v, request_id, .. }
            | Self::DevelopmentStructureDamage { v, request_id, .. }
            | Self::Release { v, request_id, .. } => (*v, request_id),
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutboundMessage {
    pub reservation_id: String,
    pub world_id: u32,
    pub player_id: u16,
    pub plugin_id: u8,
    pub payload_hex: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub v: u8,
    pub request_id: String,
    pub ok: bool,
    pub error: Option<String>,
    pub messages: Vec<OutboundMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rewards: Vec<progression::RewardEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_terminal: Option<match_flow::MatchTerminal>,
}
impl Response {
    pub fn error(request_id: String, error: &str) -> Self {
        Self {
            v: BRIDGE_VERSION,
            request_id,
            ok: false,
            error: Some(error.to_owned()),
            messages: Vec::new(),
            rewards: Vec::new(),
            match_terminal: None,
        }
    }
}
fn identifier(value: &str, min: usize) -> bool {
    (min..=64).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
fn credential(value: &str) -> bool {
    value.len() == 16
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
}
#[derive(Debug)]
pub struct ParseError {
    pub request_id: String,
    pub code: &'static str,
}

pub fn parse_request(line: &[u8]) -> Result<Request, ParseError> {
    validate_request(line).map_err(|code| {
        // Only recover a bounded valid correlation ID, never echo credentials or arbitrary JSON.
        let request_id = if line.len() <= MAX_LINE {
            serde_json::from_slice::<Value>(line)
                .ok()
                .and_then(|v| {
                    v.get("request_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .filter(|id| identifier(id, 1))
                .unwrap_or_default()
        } else {
            String::new()
        };
        ParseError { request_id, code }
    })
}

fn validate_request(line: &[u8]) -> Result<Request, &'static str> {
    if line.len() > MAX_LINE {
        return Err("line_too_long");
    }
    let request: Request = serde_json::from_slice(line).map_err(|_| "invalid_json_or_schema")?;
    request.validate()?;
    Ok(request)
}

impl Request {
    fn validate(&self) -> Result<(), &'static str> {
        let (version, request_id) = self.header();
        if version != BRIDGE_VERSION {
            return Err("unsupported_version");
        }
        if !identifier(request_id, 1) {
            return Err("invalid_request_id");
        }
        match self {
            Request::Hello { .. } => {}
            Request::ConfigureMatch {
                minimum_players,
                minimum_players_per_team,
                maximum_players,
                ..
            } => {
                if *minimum_players == 0
                    || minimum_players > maximum_players
                    || *maximum_players > MAX_MATCH_PLAYERS
                    || minimum_players_per_team.iter().any(|n| n > maximum_players)
                    || minimum_players_per_team[0].saturating_add(minimum_players_per_team[1])
                        > *maximum_players
                {
                    return Err("invalid_match_limits");
                }
            }
            Request::Reserve {
                reservation_id,
                account_id,
                session_key,
                access_key,
                character,
                ..
            } => {
                if !identifier(reservation_id, 16) {
                    return Err("invalid_reservation_id");
                }
                if *account_id == 0 {
                    return Err("invalid_account_id");
                }
                if !credential(session_key) || !credential(access_key) {
                    return Err("invalid_credentials");
                }
                if character.name.is_empty()
                    || character.name.len() > 32
                    || !character
                        .name
                        .bytes()
                        .all(|b| b.is_ascii_graphic() || b == b' ')
                {
                    return Err("invalid_character_name");
                }
                if character.account_id.is_some() != character.progression.is_some() {
                    return Err("incomplete_progression_identity");
                }
                if let Some(account) = &character.account_id
                    && (account.is_empty()
                        || account.len() > 32
                        || !account
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                        || character.character_id.is_none_or(|id| id == 0))
                {
                    return Err("invalid_progression_identity");
                }
                if let Some(state) = character.progression {
                    state.validate()?
                }
            }
            Request::Event {
                reservation_id,
                payload_hex,
                ..
            } => {
                if !identifier(reservation_id, 16) {
                    return Err("invalid_reservation_id");
                }
                validate_payload_hex(payload_hex)?;
            }
            Request::ProgressionCommitted {
                reservation_id,
                reward_id,
                progression,
                match_experience,
                ..
            } => {
                if !identifier(reservation_id, 16) {
                    return Err("invalid_reservation_id");
                }
                if reward_id.is_empty()
                    || reward_id.len() > 128
                    || !reward_id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                {
                    return Err("invalid_reward_id");
                }
                progression.validate()?;
                if *match_experience > i32::MAX as u32 {
                    return Err("invalid_match_experience");
                }
            }
            Request::DevelopmentSpawnTarget {
                reservation_id,
                spawn_id,
                position,
                bot_script,
                ..
            } => {
                if !identifier(reservation_id, 16) || !identifier(spawn_id, 1) {
                    return Err("invalid_target_spawn_identity");
                }
                position.validate()?;
                if let Some(script) = bot_script {
                    script.validate(*position)?;
                }
            }
            Request::DevelopmentMoveBot {
                reservation_id,
                movement_id,
                movement,
                ..
            } => {
                if !identifier(reservation_id, 16) || !identifier(movement_id, 1) {
                    return Err("invalid_bot_move_identity");
                }
                movement.validate()?;
            }
            Request::DevelopmentDamage {
                reservation_id,
                damage_id,
                damage,
                ..
            } => {
                if !identifier(reservation_id, 16) {
                    return Err("invalid_reservation_id");
                }
                if !identifier(damage_id, 1) {
                    return Err("invalid_damage_id");
                }
                damage.validate()?;
            }
            Request::DevelopmentArmDemolition { reservation_id, .. } => {
                if !identifier(reservation_id, 16) {
                    return Err("invalid_reservation_id");
                }
            }
            Request::DevelopmentStructureDamage {
                reservation_id,
                damage_id,
                amount_hp,
                ..
            } => {
                if !identifier(reservation_id, 16) || !identifier(damage_id, 1) {
                    return Err("invalid_structure_damage_identity");
                }
                if !amount_hp.is_finite() || *amount_hp <= 0.0 || *amount_hp > 1000.0 {
                    return Err("invalid_structure_damage_amount");
                }
            }
            Request::Release { reservation_id, .. } => {
                if !identifier(reservation_id, 16) {
                    return Err("invalid_reservation_id");
                }
            }
        }
        Ok(())
    }
}

fn validate_payload_hex(hex: &str) -> Result<(), &'static str> {
    if hex.len() > MAX_PAYLOAD * 2 {
        return Err("payload_too_long");
    }
    if !hex.len().is_multiple_of(2) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid_hex");
    }
    Ok(())
}

#[derive(PartialEq, Eq)]
struct Reservation {
    id: String,
    world: u32,
    player: u16,
    account: u32,
    session_key: String,
    access_key: String,
    character: Character,
}
struct Claimed {
    wire: Vec<u8>,
    game_info: protocol::GameInfo,
    np_identity: [u8; 36],
}
// A local history bound, not a native protocol limit. Retain immutable packets
// until acknowledgement retirement is traced; never recycle sequence numbers.
const MAX_REPLICATION_HISTORY: usize = 1024;
/// World-owned schedule, independent of any player's queue membership or life.
/// Each match side retains its own configured schedule, including shared factions.
#[derive(Clone, Copy, Debug)]
struct TeamWaveClock {
    first_deadline: Duration,
    period: Duration,
}
impl TeamWaveClock {
    fn next_after(self, now: Duration) -> Result<Duration, &'static str> {
        if now < self.first_deadline {
            return Ok(self.first_deadline);
        }
        let remainder = (now - self.first_deadline).as_nanos() % self.period.as_nanos();
        now.checked_add(self.period - Duration::from_nanos(remainder as u64))
            .ok_or("wave_clock_overflow")
    }
}
#[derive(Clone, Copy, Debug)]
enum CharacterLife {
    AwaitingFirstSpawn,
    Alive {
        generation: u32,
        health_quarter_points: u16,
    },
    AwaitingRespawn {
        generation: u32,
    },
}
impl CharacterLife {
    fn is_alive(self) -> bool {
        matches!(self, Self::Alive { .. })
    }
    fn next_generation(self) -> Option<u32> {
        match self {
            Self::AwaitingFirstSpawn => Some(1),
            Self::AwaitingRespawn { generation } => generation.checked_add(1),
            Self::Alive { .. } => None,
        }
    }
}
#[derive(Clone)]
struct PendingDamage {
    target_character_id: u32,
    target_reservation_id: Option<String>,
    id: String,
    generation: u32,
    damage: damage::ResolvedDamage,
    falling: bool,
}
enum CharacterHitProfile {
    OnFoot(ballistics::BallisticProfile),
    Retail(retail_combat::RetailBulletProfile),
    Melee(retail_melee::RetailMeleeProfile),
    Alpha(turret::AlphaMinigunProfile),
}
impl CharacterHitProfile {
    fn kind(&self) -> damage::DamageKind {
        match self {
            Self::Melee(_) => damage::DamageKind::Swipe,
            _ => damage::DamageKind::Bullet,
        }
    }
    fn item_id(&self) -> u16 {
        match self {
            Self::OnFoot(p) => p.item_id(),
            Self::Retail(p) => p.item_id(),
            Self::Melee(p) => p.item_id(),
            Self::Alpha(p) => p.item_id(),
        }
    }
    fn raw_character_damage(
        &self,
        distance: f32,
        scale: f32,
        location: u8,
    ) -> Result<armor::RawCharacterDamage, &'static str> {
        match self {
            Self::OnFoot(p) => p.raw_character_damage(distance, scale, location),
            Self::Retail(p) => p.raw_character_damage(distance, scale, location),
            Self::Melee(p) => p.raw_character_damage(distance, scale, location),
            Self::Alpha(p) => p.raw_character_damage(distance, scale, location),
        }
    }
}
#[derive(Clone)]
struct ProjectileCleanup {
    due: Duration,
    removal: projectile_replication::ProjectileRemoval,
}
struct ProjectilePublication {
    include_owner: bool,
    wire: Vec<u8>,
}
struct Active {
    wire_profile: wire_profile::WireProfile,
    reservation: Reservation,
    modifiers: skill_catalog::OwnedModifiers,
    leadership: Option<leadership_runtime::CachedLeadership>,
    /// Saved leadership level, including the native policy's fallback record.
    leadership_level: u8,
    /// Replicated magInfo nominations after retail leadership-level masking.
    nominations: u8,
    last_event: Duration,
    claimed: Option<Claimed>,
    ready_for_updates: bool,
    initial_objects: Option<Vec<u8>>,
    spawn_delta: Option<Vec<u8>>,
    life: CharacterLife,
    tracking: player_tracking::PlayerTracking,
    fall_tracker: fall_damage::Tracker,
    pending_damage: Option<PendingDamage>,
    last_damage: replication::CharacterDamage,
    healing: Option<progression::HealingSession>,
    healing_sequence: u64,
    reward_match_id: String,
    match_experience: u32,
    reward_statistics: std::collections::BTreeMap<u8, i32>,
    next_heal_allowed: Duration,
    projectiles: projectile::ProjectileFlights,
    projectile_replicas: projectile_replication::ProjectileReplicas,
    projectile_cleanup: [Option<ProjectileCleanup>; 3],
    projectile_publications: Vec<ProjectilePublication>,
    grenade_remaining: u8,
    observed_primary: Option<replication::InventoryItem>,
    observed_secondary: Option<replication::InventoryItem>,
    primary_equipped: bool,
    dummy_created: bool,
    development_target: Option<development_target::Target>,
    target_commands: std::collections::BTreeMap<
        String,
        (u32, replication::CharacterPosition, Option<BotScript>),
    >,
    dummy_health: Option<u16>,
    latest_owner_sequence: u32,
    grenade_damage: std::collections::VecDeque<PendingDamage>,
    damage_commands: std::collections::BTreeMap<String, (u32, damage::ResolvedDamage)>,
    replication_history: Vec<Vec<u8>>,
    outbound_sequence: u32,
    pending_shared: std::collections::VecDeque<Vec<u8>>,
    replication_backlogged: bool,
    objective_delta: Option<Vec<u8>>,
    objective_assigned: bool,
    objective_needs_clear: bool,
    emplacement_creation: Option<Vec<u8>>,
    mounted_seat: Option<u8>,
    seat_transitions: Vec<Vec<u8>>,
    last_exit: Option<protocol::DismountSeat>,
    match_clock_started: Option<Duration>,
    last_clock_points: i16,
    round_scores: Option<[i16; 2]>,
    pending_notifications: std::collections::VecDeque<match_flow::Publication>,
    end_announced: bool,
    results_announced: bool,
    postgame_announced: bool,
    current_inventory_handle: Option<replication::InventoryHandle>,
    selected_loadout: Option<protocol::SelectLoadout>,
    selected_spawn: Option<protocol::CharacterSpawnSelection>,
    squad_spawn: u16,
    squad_catalog: std::sync::Arc<spawn_catalog::SpawnCatalog>,
    pending_spawn_deadline: Option<Duration>,
    published_wave_deadline: Option<Duration>,
    entered_game: bool,
    owner_fields: player_tracking::ClientCharacterFields,
}

/// The writer is flushed before a request is acknowledged or state changes.
/// Once recording fails this process remains unavailable, preserving evidence.
pub struct Engine<W: Write> {
    wire_profiles: wire_profile::WireProfileCatalog,
    wire_profile: wire_profile::WireProfile,
    /// Explicit production resources bind this instance to one application.
    /// Legacy development constructors permit profile changes before admission.
    runtime_application_id: Option<u32>,
    writer: W,
    active: Option<Active>,
    peers: std::collections::BTreeMap<String, Active>,
    minimum_players: usize,
    minimum_players_per_team: [usize; 2],
    maximum_players: usize,
    match_started: bool,
    /// Wall time of the request being handled, for records made outside a plan.
    last_wall_secs: u64,
    world_epoch: Option<Duration>,
    bot_registry: bot_registry::BotRegistry,
    bot_names: bot_names::Catalog,
    world_tick_enabled: bool,
    last_world_tick: Option<Duration>,
    match_epoch: Option<Duration>,
    alpha_turret: turret::ClientTurretFields,
    alpha_turret_reporter: Option<(String, u32)>,
    team_waves: [Option<TeamWaveClock>; 2],
    shared_drain_cursor: u16,
    last_published_rosters: std::collections::BTreeMap<u16, ([u16; 8], u16)>,
    sequence: u64,
    written: u64,
    max_log: u64,
    failed: bool,
    development_control: bool,
    development_grenade_radius: bool,
    development_dummy: bool,
    development_dummy_position: Option<replication::CharacterPosition>,
    collision_world: Option<collision::CollisionWorld>,
    navmesh_world: Option<std::sync::Arc<navmesh::NavMesh>>,
    spawn_catalog: Option<spawn_catalog::SpawnCatalog>,
    parachute_waves: std::collections::BTreeMap<(u16, Duration), usize>,
    objective_catalog: Option<objective_catalog::ObjectiveCatalog>,
    objective_world: Option<objective_control::ObjectiveWorld>,
    score_clock: score_clock::Clock,
    round: match_flow::Round,
    round_number: u64,
    reward_match_id: Option<String>,
    round_participants:
        std::collections::BTreeMap<progression::ParticipantKey, progression::RoundParticipation>,
    match_terminal: Option<match_flow::MatchTerminal>,
    released_reservations: std::collections::BTreeSet<String>,
    release_order: std::collections::VecDeque<String>,
    bunker_world: Option<structures::BunkerWorld>,
    bunker_exit_policy: Option<vehicle_exit::BunkerExitPolicy>,
    pending_bunker_damage: std::collections::VecDeque<(u16, f32, String)>,
    next_repair: std::collections::BTreeMap<String, Duration>,
    item_catalog: std::sync::Arc<item_catalog::ItemCatalog>,
    skill_catalog: Option<skill_catalog::SkillCatalog>,
    leadership: Option<leadership_runtime::LeadershipRuntime>,
    fall_damage_rules: Option<fall_damage::Rules>,
    pending_rewards: std::collections::BTreeMap<String, progression::RewardEvent>,
    reward_order: std::collections::VecDeque<String>,
    committed_rewards: std::collections::BTreeMap<String, progression::CommittedReward>,
    committed_reward_order: std::collections::VecDeque<String>,
}
impl<W: Write> Engine<W> {
    pub fn with_fall_damage_rules(mut self, rules: fall_damage::Rules) -> Self {
        self.fall_damage_rules = Some(rules);
        self
    }
    pub fn with_skill_catalog(mut self, catalog: skill_catalog::SkillCatalog) -> Self {
        self.skill_catalog = Some(catalog);
        self
    }
    fn alpha_turret_authority(
        &self,
        active: &Active,
    ) -> Result<turret::AlphaTurretAuthority, &'static str> {
        turret::AlphaTurretAuthority::authorize(
            active.reservation.player,
            active.entered_game,
            active.ready_for_updates,
            active.life.is_alive(),
            active.mounted_seat,
            self.alpha_owner()
                .filter(|(_, seat)| *seat == 0)
                .map(|(player, _)| player),
        )
    }
    fn bullet_profile(
        &self,
        active: &Active,
        request: &bullet_hit::NetMessageHit,
    ) -> Result<CharacterHitProfile, &'static str> {
        if active.mounted_seat == Some(0) {
            let authority = self.alpha_turret_authority(active)?;
            let generation = match active.life {
                CharacterLife::Alive { generation, .. } => generation,
                _ => return Err("turret_requires_living_exclusive_owner"),
            };
            if self.alpha_turret_reporter.as_ref()
                != Some(&(active.reservation.id.clone(), generation))
            {
                return Err("turret_equipment_unavailable");
            }
            return Ok(CharacterHitProfile::Alpha(
                authority.minigun_profile_from_equipped_with_catalog(
                    self.alpha_turret
                        .primary
                        .ok_or("turret_equipment_unavailable")?,
                    request.gear_index,
                    request.fire_mode,
                    &self.item_catalog,
                )?,
            ));
        }
        let handle = active
            .current_inventory_handle
            .ok_or("bullet_inventory_unavailable")?;
        if active.wire_profile.application_id() == 20231 {
            let mut kit = active
                .reservation
                .character
                .replication_loadout(
                    &self.item_catalog,
                    handle.inventory_index,
                    active.wire_profile,
                )
                .ok_or("unsupported_retail_bullet_inventory")?;
            kit.handle = handle;
            if request.gear_index == 2 {
                if active.mounted_seat.is_some() {
                    return Err("melee_requires_on_foot");
                }
                let attacker_yaw = active
                    .owner_fields
                    .yaw
                    .ok_or("melee_attacker_facing_unavailable")?;
                let defender_yaw = self
                    .peers
                    .values()
                    .find(|p| (0x03800000 | u32::from(p.reservation.player)) == request.defender)
                    .and_then(|p| p.owner_fields.yaw)
                    .or_else(|| {
                        self.bot_combat_snapshots()
                            .into_iter()
                            .find(|b| (0x03800000 | u32::from(b.object_index)) == request.defender)
                            .and_then(|b| b.yaw)
                    })
                    .ok_or("melee_defender_facing_unavailable")?;
                return Ok(CharacterHitProfile::Melee(
                    retail_melee::RetailMeleeProfile::from_kit(
                        self.item_catalog.retail_combat()?,
                        &kit,
                        request.gear_index,
                        request.fire_mode,
                        active.modifiers.value("kCharModKnifeReach"),
                        attacker_yaw,
                        defender_yaw,
                    )?,
                ));
            }
            let equipped = if active.primary_equipped {
                active.observed_primary
            } else {
                active.observed_secondary
            }
            .ok_or("bullet_equipped_item_unavailable")?;
            return Ok(CharacterHitProfile::Retail(
                retail_combat::RetailBulletProfile::from_equipped(
                    self.item_catalog.retail_combat()?,
                    &kit,
                    equipped,
                    request.gear_index,
                    request.fire_mode,
                )?,
            ));
        }
        let mut kit = active
            .reservation
            .character
            .loadout(
                &self.item_catalog,
                handle.inventory_index,
                active.wire_profile,
            )
            .ok_or("unsupported_bullet_inventory")?;
        kit.handle = handle;
        kit.primary.inventory_update_id = handle.update_id;
        let equipped = if active.primary_equipped {
            active.observed_primary
        } else {
            active.observed_secondary
        }
        .ok_or("bullet_equipped_item_unavailable")?;
        Ok(CharacterHitProfile::OnFoot(
            ballistics::BallisticProfile::from_equipped_with_catalog(
                &kit,
                equipped,
                request.gear_index,
                request.fire_mode,
                &self.item_catalog,
            )?,
        ))
    }
    /// Match policy of the selected map; Alyeska Domination when no catalog is loaded.
    pub fn match_profile(&self) -> spawn_catalog::MatchProfile {
        self.spawn_catalog
            .as_ref()
            .map(|c| c.match_profile())
            .unwrap_or_default()
    }
    pub fn map_name(&self) -> &str {
        self.spawn_catalog
            .as_ref()
            .map_or(ACTIVE_MAP_NAME, |c| c.map_name())
    }
    pub fn with_objective_catalog(
        mut self,
        catalog: objective_catalog::ObjectiveCatalog,
    ) -> Result<Self, &'static str> {
        catalog.validate_match(self.map_name(), self.match_profile())?;
        if let Some(spawns) = &self.spawn_catalog {
            spawns.validate_objectives(Some(&catalog))?;
        }
        self.objective_world = Some(objective_control::ObjectiveWorld::new(&catalog));
        self.objective_catalog = Some(catalog);
        Ok(self)
    }
    fn validate_objective_configuration(
        &self,
        profile: wire_profile::WireProfile,
    ) -> Result<(), &'static str> {
        if let Some(spawns) = &self.spawn_catalog {
            spawns.validate_objectives(self.objective_catalog.as_ref())?;
        }
        objective_catalog::validate_configuration(
            self.objective_catalog.as_ref(),
            self.map_name(),
            self.match_profile(),
            profile,
        )
    }

    fn initial_objective_index(&self, squad: u16) -> Result<u16, &'static str> {
        match &self.objective_catalog {
            Some(catalog) => {
                if let Some(world) = &self.objective_world {
                    catalog.dynamic_assignment(squad, |index| world.state(index))
                } else {
                    catalog.initial_assignment(squad)
                }
            }
            None => Err("retail_objective_catalog_required"),
        }
    }
    pub fn with_spawn_catalog(mut self, catalog: spawn_catalog::SpawnCatalog) -> Self {
        self.development_dummy_position = catalog.alternate_position();
        self.spawn_catalog = Some(catalog);
        self
    }
    /// Explicit lab opt-in. Bridge is never published on a host/game port.
    pub fn with_development_grenade_radius(mut self) -> Self {
        self.development_grenade_radius = true;
        self
    }
    pub(crate) fn development_dummy_enabled(&self) -> bool {
        self.development_dummy && self.wire_profile.catalog_item_ids_valid()
    }
    /// Explicit local target; never admitted as a fake connected client.
    pub fn with_development_dummy(mut self) -> Self {
        self.development_dummy = true;
        self
    }
    /// Lab override validated before any admission. Does not alter stored Player.
    pub fn with_development_dummy_position(
        mut self,
        position: replication::CharacterPosition,
    ) -> Result<Self, &'static str> {
        dummy::respawn_at_with_catalog_and_profile(
            2,
            0,
            position,
            &self.item_catalog,
            self.wire_profile,
        )?;
        self.development_dummy_position = Some(position);
        Ok(self)
    }
    pub fn with_development_control(mut self) -> Self {
        self.development_control = true;
        self
    }
    /// Disk-loaded geometry is queried for evidence only until map placement,
    /// filters and dynamic geometry have independent client validation.
    pub fn with_collision_world(mut self, world: collision::CollisionWorld) -> Self {
        self.collision_world = Some(world);
        self
    }
    pub fn with_navmesh_world(mut self, mesh: navmesh::NavMesh) -> Self {
        self.navmesh_world = Some(std::sync::Arc::new(mesh));
        self
    }
    pub fn into_writer(self) -> W {
        self.writer
    }
    fn record(&mut self, mut value: Value, wall_secs: u64) -> Result<(), &'static str> {
        if self.failed {
            return Err("recorder_unavailable");
        }
        value["sequence"] = json!(self.sequence + 1);
        value["unix_seconds"] = json!(wall_secs);
        let mut bytes = serde_json::to_vec(&value).map_err(|_| "record_encode_failed")?;
        bytes.push(b'\n');
        if self.written + bytes.len() as u64 > self.max_log {
            self.failed = true;
            return Err("log_limit_reached");
        }
        if self
            .writer
            .write_all(&bytes)
            .and_then(|_| self.writer.flush())
            .is_err()
        {
            self.failed = true;
            return Err("recorder_unavailable");
        }
        self.written += bytes.len() as u64;
        self.sequence += 1;
        Ok(())
    }
    pub fn handle(&mut self, request: Request, now: Duration, wall_secs: u64) -> Response {
        self.last_wall_secs = wall_secs;
        let request_id = request.header().1.to_owned();
        // Public callers can construct Request without going through parse_request.
        if let Err(code) = request.validate() {
            return Response::error(
                if identifier(&request_id, 1) {
                    request_id
                } else {
                    String::new()
                },
                code,
            );
        }
        // Public validation above guarantees bounded, valid hex. Own the bytes
        // here so every wrapper and planner can borrow one complete decode.
        let bytes = match &request {
            Request::Event { payload_hex, .. } => protocol::decode_hex(payload_hex),
            _ => Vec::new(),
        };
        let decoded = match &request {
            Request::Event { player_id, .. } => Some(DecodedEvent::with_profile(
                &bytes,
                *player_id,
                self.wire_profile,
            )),
            _ => None,
        };
        let terminal_allowed = matches!(
            &request,
            Request::Event { .. } | Request::Release { .. } | Request::ProgressionCommitted { .. }
        );
        let result = self.apply_shared(request, decoded.as_ref(), now, wall_secs);
        match result {
            Ok(messages) => Response {
                v: BRIDGE_VERSION,
                request_id,
                ok: true,
                error: None,
                messages,
                rewards: self.pending_reward_batch(),
                match_terminal: if terminal_allowed {
                    self.match_terminal.clone()
                } else {
                    None
                },
            },
            Err(code) => Response::error(request_id, code),
        }
    }
    fn apply(
        &mut self,
        request: Request,
        decoded: Option<&DecodedEvent<'_>>,
        now: Duration,
        wall_secs: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        // Process owner fields and projectile reports even when world durability
        // needs a reply. The bridge permits one reply per event, so defer health
        // publication while a spawn/replay packet has priority. Periodic clocks
        // yield to pending durability so idle one-update-per-second clients
        // cannot starve world health changes.
        let health_update = matches!(
            &request,
            Request::Event {
                transport: Transport::Udp,
                rt_type: 0x3d,
                plugin_id: Some(1),
                ..
            }
        ) && decoded.and_then(DecodedEvent::owner_update).is_some_and(
            |(update, _)| update.header.client_index == 0 && update.header.nak_sequence == 0,
        );
        let messages = self.apply_core(request, decoded, now, wall_secs)?;
        if messages.is_empty()
            && health_update
            && let Some(decoded) = decoded
        {
            if let Some(messages) = self.structure_event(&decoded.message, now, wall_secs)? {
                return Ok(messages);
            }
            if let Some(messages) = self.tick_bunkers(now, wall_secs)? {
                return Ok(messages);
            }
        }
        Ok(messages)
    }
    fn apply_core(
        &mut self,
        request: Request,
        decoded: Option<&DecodedEvent<'_>>,
        now: Duration,
        wall_secs: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        if self.failed {
            return Err("recorder_unavailable");
        }
        if self
            .active
            .as_ref()
            .is_some_and(|a| now.saturating_sub(a.last_event) >= IDLE_TIMEOUT)
        {
            let id = self.active.as_ref().unwrap().reservation.id.clone();
            self.record(json!({"op":"expired","reservation_id":id}), wall_secs)?;
            let removed = self.active.take().unwrap();
            self.remember_round_participant(&removed);
            if self.peers.is_empty() {
                self.reset_objective_world();
                self.clear_bot_world();
                self.team_waves = [None; 2];
                self.shared_drain_cursor = 0;
                self.last_published_rosters.clear();
                self.world_epoch = None;
                self.match_epoch = None;
                self.match_started = self.starts_without_quorum();
            }
        }
        match request {
            Request::Hello { .. } => self
                .record(
                    json!({"op":"ready","bridge_version":BRIDGE_VERSION}),
                    wall_secs,
                )
                .map(|_| Vec::new()),
            Request::ProgressionCommitted { .. } => unreachable!("handled before actor selection"),
            Request::ConfigureMatch {
                minimum_players,
                minimum_players_per_team,
                maximum_players,
                application_id,
                ..
            } => {
                if self
                    .runtime_application_id
                    .is_some_and(|selected| selected != application_id)
                {
                    return Err("runtime_application_mismatch");
                }
                if minimum_players == 0
                    || minimum_players > maximum_players
                    || maximum_players > self.admission_capacity_for(application_id)
                {
                    return Err("invalid_match_limits");
                }
                self.validate_team_quorum(minimum_players_per_team, maximum_players)?;
                if self.active.is_some() || !self.peers.is_empty() {
                    return Err("match_configuration_locked");
                }
                let profile = self.wire_profiles.profile(application_id)?;
                if application_id == 20231 {
                    self.item_catalog
                        .validate_bootstrap_faction(self.match_profile().defending_faction)?;
                }
                self.validate_objective_configuration(profile)?;
                if maximum_players > 1 && self.development_dummy && profile.catalog_item_ids_valid()
                {
                    return Err("dummy_slot_conflicts_with_multiplayer");
                }
                self.record(json!({"op":"match_configured","application_id":application_id,"object_type_bits":profile.object_type_bits(),"header_bits":profile.header_bits(),"inventory_item_bits":profile.inventory_item_bits(),"stat_encoding":profile.stat_encoding(),"minimum_players":minimum_players,"minimum_players_per_team":minimum_players_per_team,"maximum_players":maximum_players}),wall_secs)?;
                self.minimum_players = minimum_players;
                self.minimum_players_per_team = minimum_players_per_team;
                self.wire_profile = profile;
                self.maximum_players = maximum_players;
                self.match_started = self.starts_without_quorum();
                Ok(Vec::new())
            }
            Request::Reserve {
                reservation_id,
                world_id,
                player_id,
                account_id,
                session_key,
                access_key,
                character,
                ..
            } => {
                self.validate_objective_configuration(self.wire_profile)?;
                if self.active.is_none() && self.peers.is_empty() {
                    if let Some(world) = &mut self.bunker_world {
                        world.reset();
                    }
                    self.next_repair.clear();
                    self.pending_bunker_damage.clear();
                }
                let candidate = Reservation {
                    id: reservation_id,
                    world: world_id,
                    player: player_id,
                    account: account_id,
                    session_key,
                    access_key,
                    character,
                };
                if usize::from(player_id) >= self.maximum_players {
                    return Err("invalid_player_slot");
                }
                let modifiers = match candidate.character.skills {
                    Some(owned) => self
                        .skill_catalog
                        .as_ref()
                        .ok_or("skill_catalog_unavailable")?
                        .resolve(candidate.character.faction, &owned)?,
                    None => skill_catalog::OwnedModifiers::default(),
                };
                if let Some(saved) = &candidate.character.loadouts {
                    if candidate.character.character_id.is_none_or(|id| id == 0) {
                        return Err("invalid_saved_character_id");
                    }
                    if self.wire_profile.application_id() == 20231
                        && let Some(armory) = self.item_catalog.retail_armory()
                    {
                        self.item_catalog.validate_saved_item_bindings(saved)?;
                        armory.validate_saved(candidate.character.faction, saved)?;
                    } else {
                        saved.validate(&self.item_catalog)?;
                    }
                }
                if let Some(active) = &self.active {
                    return if active.reservation == candidate {
                        Ok(Vec::new())
                    } else {
                        Err("reservation_conflict")
                    };
                }
                // Validate and allocate before recording/acknowledging admission.
                let mut squad_catalog = self.allocate_squad(candidate.character.faction)?;
                let shared_spawn = self
                    .squad_spawn_for(squad_catalog.squad_index())
                    .ok_or("missing_squad_spawn")?;
                squad_catalog.set_runtime_default(shared_spawn)?;
                self.record(
                    json!({"op":"reserved","reservation_id":candidate.id,"world_id":world_id,
                    "player_id":player_id,"account_id":account_id,"character":candidate.character,
                    "team":squad_catalog.squad_index()/16,"squad_index":squad_catalog.squad_index()}),
                    wall_secs,
                )?;
                if self.peers.is_empty() {
                    self.round_number = self
                        .round_number
                        .checked_add(1)
                        .ok_or("round_number_exhausted")?;
                }
                let reward_match_id = self
                    .reward_match_id
                    .get_or_insert_with(|| {
                        progression::round_identity(&candidate, self.round_number)
                    })
                    .clone();
                let participation = progression::participant_key(&candidate)
                    .and_then(|key| self.round_participants.get(&key))
                    .cloned()
                    .unwrap_or_default();
                let nominations = self.initial_nominations(&candidate.character);
                let leadership_level = self
                    .leadership_record(&candidate.character)
                    .or(candidate.character.leadership)
                    .map_or(1, |record| record.leadership_level);
                self.active = Some(Active {
                    wire_profile: self.wire_profile,
                    reservation: candidate,
                    modifiers,
                    leadership: None,
                    leadership_level,
                    nominations,
                    last_event: now,
                    claimed: None,
                    ready_for_updates: false,
                    initial_objects: None,
                    spawn_delta: None,
                    life: CharacterLife::AwaitingFirstSpawn,
                    tracking: player_tracking::PlayerTracking::default(),
                    fall_tracker: fall_damage::Tracker::default(),
                    pending_damage: None,
                    last_damage: replication::CharacterDamage {
                        last_location_and_type: 0,
                        health_quarter_points: 400,
                        conditions: 0,
                        last_attacker: 0,
                        last_source_object: 0,
                        last_user_data: 0,
                    },
                    healing: None,
                    healing_sequence: 0,
                    reward_match_id,
                    match_experience: participation.match_experience,
                    reward_statistics: participation.statistics,
                    next_heal_allowed: Duration::ZERO,
                    projectiles: projectile::ProjectileFlights::default(),
                    projectile_replicas: projectile_replication::ProjectileReplicas::with_profile(
                        self.wire_profile,
                    ),
                    projectile_cleanup: [None, None, None],
                    projectile_publications: Vec::new(),
                    grenade_remaining: 0,
                    observed_primary: None,
                    observed_secondary: None,
                    primary_equipped: true,
                    dummy_created: false,
                    development_target: None,
                    target_commands: std::collections::BTreeMap::new(),
                    dummy_health: None,
                    latest_owner_sequence: 0,
                    grenade_damage: std::collections::VecDeque::new(),
                    damage_commands: std::collections::BTreeMap::new(),
                    replication_history: Vec::new(),
                    outbound_sequence: 0,
                    pending_shared: std::collections::VecDeque::new(),
                    replication_backlogged: false,
                    objective_delta: None,
                    objective_assigned: false,
                    objective_needs_clear: false,
                    emplacement_creation: None,
                    mounted_seat: None,
                    seat_transitions: Vec::new(),
                    last_exit: None,
                    match_clock_started: None,
                    last_clock_points: self.match_profile().maximum_points[0],
                    round_scores: None,
                    pending_notifications: Default::default(),
                    end_announced: false,
                    results_announced: false,
                    postgame_announced: false,
                    current_inventory_handle: None,
                    selected_loadout: None,
                    selected_spawn: None,
                    squad_spawn: shared_spawn,
                    squad_catalog: std::sync::Arc::new(squad_catalog),
                    pending_spawn_deadline: None,
                    published_wave_deadline: None,
                    entered_game: false,
                    owner_fields: player_tracking::ClientCharacterFields::default(),
                });
                Ok(Vec::new())
            }
            Request::DevelopmentArmDemolition {
                reservation_id,
                world_id,
                player_id,
                object_index,
                ..
            } => self.seed_development_demolition(
                &reservation_id,
                world_id,
                player_id,
                object_index,
                now,
                wall_secs,
            ),
            Request::DevelopmentStructureDamage {
                reservation_id,
                world_id,
                player_id,
                damage_id,
                target_object_id,
                amount_hp,
                ..
            } => self.queue_development_bunker_damage(
                &reservation_id,
                world_id,
                player_id,
                &damage_id,
                target_object_id,
                amount_hp,
                wall_secs,
            ),
            Request::DevelopmentSpawnTarget {
                reservation_id,
                world_id,
                player_id,
                spawn_id,
                expected_generation,
                position,
                bot_script,
                ..
            } => self.queue_development_target(
                &reservation_id,
                world_id,
                player_id,
                spawn_id,
                expected_generation,
                position,
                bot_script,
                wall_secs,
            ),
            Request::DevelopmentMoveBot {
                reservation_id,
                world_id,
                player_id,
                movement_id,
                expected_owner_generation,
                expected_bot_generation,
                movement,
                ..
            } => self.queue_development_bot_move(
                &reservation_id,
                world_id,
                player_id,
                movement_id,
                expected_owner_generation,
                expected_bot_generation,
                movement,
                now,
                wall_secs,
            ),
            Request::DevelopmentDamage {
                reservation_id,
                world_id,
                player_id,
                damage_id,
                target_character_id,
                expected_generation,
                damage,
                ..
            } => {
                if !self.development_control {
                    return Err("development_control_disabled");
                }
                damage.validate()?;
                let active = self.active.as_ref().ok_or("no_reservation")?;
                if active.reservation.id != reservation_id
                    || active.reservation.world != world_id
                    || active.reservation.player != player_id
                {
                    return Err("identity_mismatch");
                }
                if target_character_id != (0x03800000 | u32::from(active.reservation.player)) {
                    return Err("unowned_damage_target");
                }
                if let Some(previous) = active.damage_commands.get(&damage_id) {
                    return if *previous == (expected_generation, damage) {
                        Ok(Vec::new())
                    } else {
                        Err("damage_id_conflict")
                    };
                }
                let CharacterLife::Alive { generation, .. } = active.life else {
                    return Err("damage_requires_living_character");
                };
                if generation != expected_generation {
                    return Err("damage_life_mismatch");
                }
                if active.mounted_seat.is_some() {
                    return Err("mounted_damage_not_supported");
                }
                if !active.entered_game || !active.ready_for_updates {
                    return Err("damage_requires_ready_character");
                }
                if active.pending_damage.is_some() {
                    return Err("damage_queue_full");
                }
                if active.damage_commands.len() >= 64 {
                    return Err("damage_command_limit");
                }
                if self.maximum_players == 1
                    && !self.world_tick_enabled
                    && active.replication_history.len() >= MAX_REPLICATION_HISTORY
                {
                    return Err("replication_history_limit");
                }
                self.record(json!({"op":"development_damage_queued","reservation_id":reservation_id,"world_id":world_id,"player_id":player_id,
                    "damage_id":damage_id,"life_generation":generation,"damage":damage}),wall_secs)?;
                let active = self.active.as_mut().unwrap();
                active
                    .damage_commands
                    .insert(damage_id.clone(), (generation, damage));
                active.pending_damage = Some(PendingDamage {
                    falling: false,
                    target_character_id: (0x03800000 | u32::from(active.reservation.player)),
                    target_reservation_id: Some(active.reservation.id.clone()),
                    id: damage_id,
                    generation,
                    damage,
                });
                Ok(Vec::new())
            }
            Request::Event {
                reservation_id,
                world_id,
                player_id,
                transport,
                direction,
                rt_type,
                plugin_id,
                payload_hex,
                ..
            } => self.handle_event(
                events::EventInput {
                    reservation_id,
                    world_id,
                    player_id,
                    transport,
                    direction,
                    rt_type,
                    plugin_id,
                    payload_hex,
                },
                decoded.expect("validated Event has a decoded context"),
                now,
                wall_secs,
            ),

            Request::Release { reservation_id, .. } => {
                if let Some(active) = &self.active {
                    if active.reservation.id != reservation_id {
                        return Err("identity_mismatch");
                    }
                    self.record(
                        json!({"op":"released","reservation_id":reservation_id}),
                        wall_secs,
                    )?;
                    self.released_reservations.insert(reservation_id.clone());
                    self.release_order.push_back(reservation_id);
                    while self.release_order.len() > 512 {
                        if let Some(old) = self.release_order.pop_front() {
                            self.released_reservations.remove(&old);
                        }
                    }
                    let removed = self.active.take().unwrap();
                    self.remember_round_participant(&removed);
                    if self.peers.is_empty() {
                        // Horizon settles pending release ACKs before a new cohort.
                        // Retire identities only while this shared world exists.
                        self.released_reservations.clear();
                        self.release_order.clear();
                        self.reset_objective_world();
                        self.clear_bot_world();
                        self.team_waves = [None; 2];
                        self.shared_drain_cursor = 0;
                        self.last_published_rosters.clear();
                        self.world_epoch = None;
                        self.match_epoch = None;
                        self.match_started = self.starts_without_quorum();
                    }
                }
                Ok(Vec::new())
            }
        }
    }
}

/// Read exactly one bounded line with an absolute deadline, including slow input.
pub fn read_line(stream: &mut TcpStream, timeout: Duration) -> io::Result<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    let mut line = Vec::with_capacity(1024);
    let mut buffer = [0u8; 2048];
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "request deadline"))?;
        stream.set_read_timeout(Some(remaining))?;
        let n = stream.read(&mut buffer)?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "missing newline",
            ));
        }
        if let Some(end) = buffer[..n].iter().position(|b| *b == b'\n') {
            if line.len() + end > MAX_LINE {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
            }
            // A connection carries one operation. Reject pipelined bytes in this read.
            if end + 1 != n {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "one request per connection",
                ));
            }
            line.extend_from_slice(&buffer[..end]);
            return Ok(line);
        }
        if line.len() + n > MAX_LINE {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
        }
        line.extend_from_slice(&buffer[..n]);
    }
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum BulletCollisionAudit {
    NotLoaded,
    MapMismatch {
        supplied_map: String,
        expected_map: String,
    },
    QueryFailed {
        error: String,
    },
    Queried {
        metadata: collision::WorldMetadata,
        nearest: Option<collision::RayHit>,
        authority: &'static str,
    },
}
fn collision_audit(
    world: Option<&collision::CollisionWorld>,
    expected_map: &str,
    start: [f32; 3],
    end: [f32; 3],
) -> BulletCollisionAudit {
    let Some(world) = world else {
        return BulletCollisionAudit::NotLoaded;
    };
    if world.metadata().map_name != expected_map {
        return BulletCollisionAudit::MapMismatch {
            supplied_map: world.metadata().map_name.clone(),
            expected_map: expected_map.to_owned(),
        };
    }
    match world.raycast_segment(start.map(f64::from), end.map(f64::from)) {
        Ok(query) => BulletCollisionAudit::Queried {
            metadata: query.metadata.clone(),
            nearest: query.nearest,
            authority: "audit_only",
        },
        Err(error) => BulletCollisionAudit::QueryFailed {
            error: error.to_string(),
        },
    }
}
