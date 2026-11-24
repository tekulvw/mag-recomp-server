//! Explicit retail static supply sessions for ordinary human healing and grenade refill.
use super::*;
use serde::Deserialize;
use std::{collections::BTreeSet, path::Path};

const MAX_BYTES: u64 = 65_536;
const FAMILY_PREFIX: u32 = 0x2a80_0000;
#[cfg(test)]
const PANAMA: &str = "mag_panama_military_base_final";

fn authored_provider_count(map: &str, mode: i8) -> Option<usize> {
    match (map, mode) {
        ("mag_alaska_subpen_final", 1) => Some(10),
        ("mag_panama_military_base_final", 1) => Some(9),
        ("mag_voz_final", 1) => Some(11),
        ("mag_baku_final" | "mag_canal_final", 2) => Some(20),
        ("mag_unalaska_final", 2) => Some(21),
        _ => None,
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Catalog {
    schema_version: u32,
    format: String,
    application_id: u32,
    map: String,
    mode: i8,
    entity_family: u8,
    attacking_faction: u8,
    spawn_sha256: String,
    collision_sha256: String,
    source: Source,
    providers: Vec<Provider>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    retail_elf_sha256: String,
    level_archive_sha256: String,
    clv_sha256: String,
    clv_member: String,
    id_scope: String,
    allocation_pool_capacity: u16,
    allocation_proof_sha256: String,
    definition: Definition,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Definition {
    offset: u32,
    token: u32,
    sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Provider {
    packed_target: u32,
    low16: u16,
    tick_rate_seconds: f32,
    percent_refresh_rate: f32,
    #[serde(rename = "box")]
    bounds: ProviderBox,
    source: Placement,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Placement {
    instance_offset: u32,
    instance_name_token: u32,
    instance_sha256: String,
    prefab_path: [u32; 2],
    parent_instance_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderBox {
    center: [f64; 3],
    axes: [[f64; 3]; 3],
    euler_xyz_degrees: [f64; 3],
    scales_xyz: [f64; 3],
    start_tolerance_m: f64,
    unit_half_extent: f64,
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn dot(left: [f64; 3], right: [f64; 3]) -> f64 {
    left.into_iter().zip(right).map(|(a, b)| a * b).sum()
}

impl ProviderBox {
    fn validate(&self) -> bool {
        self.center.into_iter().all(f64::is_finite)
            && self.euler_xyz_degrees.into_iter().all(f64::is_finite)
            && self
                .scales_xyz
                .into_iter()
                .all(|scale| scale.is_finite() && scale > 0.0)
            && self.start_tolerance_m == 1.0
            && self.unit_half_extent == 0.5
            && self
                .axes
                .iter()
                .flatten()
                .zip(
                    bot_resupply_catalog::authored_axes(self.euler_xyz_degrees)
                        .iter()
                        .flatten(),
                )
                .all(|(expected, actual)| (expected - actual).abs() < 1e-6)
    }

    fn contains(&self, position: [f64; 3]) -> bool {
        let displacement = std::array::from_fn(|i| position[i] - self.center[i]);
        self.axes.iter().zip(self.scales_xyz).all(|(axis, scale)| {
            dot(displacement, *axis).abs() <= scale * self.unit_half_extent + self.start_tolerance_m
        })
    }
}

impl Catalog {
    pub(crate) fn load(path: &Path) -> std::io::Result<Self> {
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)?;
        Self::from_bytes(&bytes).map_err(std::io::Error::other)
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() as u64 > MAX_BYTES {
            return Err("static_resupply_catalog_size_limit");
        }
        let catalog: Self =
            serde_json::from_slice(bytes).map_err(|_| "invalid_static_resupply_catalog")?;
        let source = &catalog.source;
        if catalog.schema_version != 1
            || catalog.format != "mag_retail_human_static_resupply"
            || catalog.application_id != 20231
            || authored_provider_count(&catalog.map, catalog.mode) != Some(catalog.providers.len())
            || catalog.entity_family != 42
            || catalog.attacking_faction > 2
            || source.id_scope != "retail_fresh_successful_compiled_load"
            || source.allocation_pool_capacity != 32
            || source.clv_member != format!("/run/mag/ps3/levels/{0}/{0}.clv", catalog.map)
            || source.definition.offset == 0
            || source.definition.token == 0
            || ![
                &catalog.spawn_sha256,
                &catalog.collision_sha256,
                &source.retail_elf_sha256,
                &source.level_archive_sha256,
                &source.clv_sha256,
                &source.allocation_proof_sha256,
                &source.definition.sha256,
            ]
            .into_iter()
            .all(|v| digest(v))
        {
            return Err("invalid_static_resupply_catalog");
        }
        let mut offsets = BTreeSet::new();
        let mut names = BTreeSet::new();
        let mut paths = BTreeSet::new();
        for (index, provider) in catalog.providers.iter().enumerate() {
            let placement = &provider.source;
            if usize::from(provider.low16) != index
                || provider.packed_target != FAMILY_PREFIX | u32::from(provider.low16)
                || !provider.tick_rate_seconds.is_finite()
                || provider.tick_rate_seconds <= 0.0
                || !(0.0..=100.0).contains(&provider.percent_refresh_rate)
                || !provider.bounds.validate()
                || placement.instance_offset == 0
                || placement.instance_name_token == 0
                || !digest(&placement.instance_sha256)
                || !digest(&placement.parent_instance_sha256)
                || !offsets.insert(placement.instance_offset)
                || !names.insert(placement.instance_name_token)
                || !paths.insert(placement.prefab_path)
            {
                return Err("invalid_static_resupply_provider");
            }
        }
        Ok(catalog)
    }

    pub(crate) fn bind(
        &self,
        spawn: &spawn_catalog::SpawnCatalog,
        collision: Option<&collision::CollisionWorld>,
        objectives: &objective_catalog::ObjectiveCatalog,
        items: &item_catalog::ItemCatalog,
    ) -> Result<(), &'static str> {
        let world = collision.ok_or("static_resupply_requires_collision")?;
        let profile = spawn.match_profile();
        if spawn.map_name() != self.map
            || world.metadata().map_name != self.map
            || profile.game_type != self.mode
            || profile.attacking_faction != self.attacking_faction
            || spawn.source_sha256() != Some(self.spawn_sha256.as_str())
            || world.source_sha256() != self.collision_sha256
            || objectives.source().source_image_sha256 != self.source.retail_elf_sha256
        {
            return Err("static_resupply_profile_source_mismatch");
        }
        items.retail_combat()?;
        Ok(())
    }

    fn provider(&self, target: u32) -> Option<&Provider> {
        let index = target as u16;
        self.providers
            .get(usize::from(index))
            .filter(|provider| provider.packed_target == target)
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Session {
    pub(super) target: u32,
    generation: u32,
    round: u64,
    elapsed: f32,
    interval: f32,
    updated: Duration,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Change {
    Start(Session),
    Stop(u32),
}

fn interval(base: f32, modifier: f32) -> Result<f32, &'static str> {
    let interval = base.mul_add(modifier, base);
    if interval.is_finite() && interval > 0.0 {
        Ok(interval)
    } else {
        Err("invalid_static_resupply_interval")
    }
}

/// The resupplied item and its inventory cap in each equipment gear.
fn equipped_grenades<W: Write>(actor: &Active, engine: &Engine<W>) -> [Option<(u16, u8)>; 2] {
    crate::GRENADE_GEARS.map(|gear| equipped_grenade(actor, engine, gear))
}

fn equipped_grenade<W: Write>(actor: &Active, engine: &Engine<W>, gear: u8) -> Option<(u16, u8)> {
    let kit = actor.equipped_loadout()?;
    let item = kit
        .inventories
        .get(usize::from(kit.handle.inventory_index))?
        .slots[usize::from(gear)]?;
    if engine
        .gas_grenades
        .as_deref()
        .and_then(|catalog| catalog.variant(actor.reservation.character.faction, item))
        .is_some()
        && !crate::gas_grenades::owns_poison_skill(actor)
    {
        return None;
    }
    // Exactly the same configured item/faction capability authorizes launch.
    let cap = engine
        .grenade_parameters(actor.reservation.character.faction, item)?
        .initial_count;
    (cap > 0).then_some((item, cap))
}

fn living_generation(actor: &Active) -> Option<u32> {
    match actor.life {
        CharacterLife::Alive {
            generation,
            health_quarter_points,
        } if health_quarter_points > 0
            && actor.ready_for_updates
            && actor.entered_game
            && actor.initial_objects.is_some()
            && !actor.is_mounted() =>
        {
            Some(generation)
        }
        _ => None,
    }
}

fn inside(actor: &Active, provider: &Provider) -> bool {
    actor.tracking.sample.as_ref().is_some_and(|sample| {
        let p = sample.position;
        provider.bounds.contains([p.x, p.y, p.z].map(f64::from))
    })
}

impl<W: Write> Engine<W> {
    fn static_resupply_catalog(&self) -> Option<&Catalog> {
        self.acquisition_gameplay
            .as_ref()
            .and_then(|world| world.setup.human_static_resupply.as_deref())
            .or_else(|| {
                self.domination_gameplay
                    .as_ref()?
                    .state
                    .human_static_resupply
                    .as_deref()
            })
    }

    pub(super) fn plan_static_resupply(
        &self,
        actor: &Active,
        request: protocol::StaticResupply,
        now: Duration,
        leadership: &leadership_runtime::LeadershipPlan,
    ) -> Result<Option<Change>, &'static str> {
        use protocol::StaticResupplyPhase;
        let catalog = self
            .static_resupply_catalog()
            .ok_or("static_resupply_not_configured")?;
        let provider = catalog
            .provider(request.target)
            .ok_or("unknown_static_resupply_provider")?;
        let existing = actor
            .static_resupply
            .iter()
            .find(|session| session.target == request.target);
        if request.phase == StaticResupplyPhase::Stop {
            return existing
                .map(|_| Some(Change::Stop(request.target)))
                .ok_or("static_resupply_session_not_owned");
        }
        if !self.match_started || self.match_terminal.is_some() || !self.round_combat_allowed(now) {
            return Err("static_resupply_round_inactive");
        }
        let generation = living_generation(actor).ok_or("static_resupply_character_ineligible")?;
        if provider.percent_refresh_rate == 0.0
            && equipped_grenades(actor, self).iter().all(Option::is_none)
            && !self
                .infantry_rockets
                .as_deref()
                .is_some_and(|catalog| infantry_rocket_runtime::State::can_resupply(actor, catalog))
            && !self
                .guided_launchers
                .as_deref()
                .is_some_and(|c| infantry_rocket_runtime::State::can_resupply_guided(actor, c))
            && !self
                .underbarrel_launchers
                .as_deref()
                .is_some_and(|catalog| {
                    infantry_rocket_runtime::State::can_resupply_underbarrel(actor, catalog)
                })
        {
            return Err("static_resupply_ordinary_frag_required");
        }
        if !inside(actor, provider) {
            return Err("static_resupply_outside_provider");
        }
        if existing.is_some_and(|session| {
            session.generation == generation && session.round == self.round_number
        }) {
            return Ok(None);
        }
        Ok(Some(Change::Start(Session {
            target: request.target,
            generation,
            round: self.round_number,
            elapsed: 0.0,
            interval: interval(
                provider.tick_rate_seconds,
                leadership.modifiers(actor).value("kCharModResupplyRate"),
            )?,
            updated: now,
        })))
    }

    pub(super) fn tick_human_static_resupply(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<(), &'static str> {
        let Some(catalog) = self.static_resupply_catalog() else {
            return Ok(());
        };
        if !self
            .active
            .iter()
            .chain(self.peers.values())
            .any(|actor| !actor.static_resupply.is_empty())
        {
            return Ok(());
        }
        let round_active =
            self.match_started && self.match_terminal.is_none() && self.round_combat_allowed(now);
        let leadership = self.plan_leadership(now)?;
        let mut planned = Vec::new();
        let mut updates = Vec::new();
        let mut audit = Vec::new();
        for actor in self.active.iter().chain(self.peers.values()) {
            if actor.static_resupply.is_empty() {
                continue;
            }
            let generation = living_generation(actor);
            let frags = equipped_grenades(actor, self);
            let launcher = self
                .infantry_rockets
                .as_deref()
                .filter(|catalog| infantry_rocket_runtime::State::can_resupply(actor, catalog));
            let guided = self
                .guided_launchers
                .as_deref()
                .filter(|c| infantry_rocket_runtime::State::can_resupply_guided(actor, c));
            let underbarrel = self.underbarrel_launchers.as_deref().filter(|catalog| {
                infantry_rocket_runtime::State::can_resupply_underbarrel(actor, catalog)
            });
            let mut launcher_state = None;
            let modifiers = leadership.modifiers(actor);
            let mut health_update = None;
            let mut sessions = Vec::with_capacity(actor.static_resupply.len());
            let mut remaining = crate::GRENADE_GEARS.map(|gear| actor.grenade_count(gear));
            let mut changed = false;
            for session in &actor.static_resupply {
                let Some(provider) = catalog.provider(session.target).filter(|provider| {
                    round_active
                        && session.round == self.round_number
                        && generation == Some(session.generation)
                        && inside(actor, provider)
                }) else {
                    changed = true;
                    audit.push(json!({"reservation":actor.reservation.id,"provider":session.target,"operation":"cancel"}));
                    continue;
                };
                if provider.percent_refresh_rate == 0.0
                    && frags.iter().all(Option::is_none)
                    && launcher.is_none()
                    && underbarrel.is_none()
                    && guided.is_none()
                {
                    changed = true;
                    audit.push(json!({"reservation":actor.reservation.id,"provider":session.target,"operation":"cancel"}));
                    continue;
                }
                let elapsed = session.elapsed + now.saturating_sub(session.updated).as_secs_f32();
                if elapsed < session.interval || now <= session.updated {
                    sessions.push(*session);
                    continue;
                }
                let mut next = *session;
                next.elapsed = elapsed - session.interval;
                next.interval = interval(
                    provider.tick_rate_seconds,
                    modifiers.value("kCharModResupplyRate"),
                )?;
                next.updated = now;
                let before = health_update.unwrap_or(actor.last_damage);
                let heal = if provider.percent_refresh_rate > 0.0 {
                    modifiers.max_health_quarter_points()?;
                    static_supply_heal::apply(
                        before,
                        modifiers.max_health_hp(),
                        provider.percent_refresh_rate,
                    )
                } else {
                    None
                };
                if let Some(model) = heal {
                    health_update = Some(model);
                }
                let after: [u8; 2] =
                    std::array::from_fn(|gear| frags[gear].map_or(remaining[gear], |(_, cap)| cap));
                if launcher_state.is_none()
                    && let Some(catalog) = launcher
                {
                    launcher_state = infantry_rocket_runtime::State::refill_for(actor, catalog)?;
                }
                if let Some(catalog) = guided {
                    launcher_state = infantry_rocket_runtime::State::refill_guided_for(
                        actor,
                        catalog,
                        launcher_state,
                    )?;
                }
                if let Some(catalog) = underbarrel {
                    launcher_state = infantry_rocket_runtime::State::refill_underbarrel_for(
                        actor,
                        catalog,
                        launcher_state,
                    )?;
                }
                audit.push(json!({"reservation":actor.reservation.id,"provider":session.target,"operation":"pulse",
                    "world":actor.reservation.world,"round":session.round,"generation":session.generation,
                    "item":frags[0].map(|(item, _)| item),"before":remaining[0],"after":after[0],
                    "gear5_item":frags[1].map(|(item, _)| item),"gear5_before":remaining[1],"gear5_after":after[1],
                    "interval_seconds":next.interval,
                    "health_before":before.health_quarter_points,
                    "health_after":heal.map(|model| model.health_quarter_points),
                    "launcher_refilled":launcher_state.is_some()}));
                remaining = after;
                sessions.push(next);
                changed = true;
            }
            if changed {
                if let Some(damage) = health_update {
                    let packet = replication::CharacterDamageUpdate {
                        sequence: 2,
                        timestamp_ms: now.as_millis() as u32,
                        object_index: actor.reservation.player,
                        damage,
                        forced_state: None,
                    }
                    .encode_with_profile(self.wire_profile)?;
                    updates.push(packet);
                }
                planned.push((
                    actor.reservation.id.clone(),
                    sessions,
                    remaining,
                    health_update.zip(generation),
                    launcher_state,
                ));
            }
        }
        if planned.is_empty() {
            return Ok(());
        }
        // Shared FIFO capacity excludes separately reserved Hospital bootstrap
        // frames. Deferral still lets the scheduler drain that FIFO this tick.
        if !updates.is_empty()
            && self.active.iter().chain(self.peers.values()).any(|actor| {
                actor.initial_objects.is_some()
                    && (actor.replication_backlogged
                        || actor.ordinary_shared_len().saturating_add(updates.len())
                            > multiplayer::MAX_SHARED_QUEUE)
            })
        {
            return Ok(());
        }
        self.record(
            json!({"op":"human_static_resupply_tick","timestamp_ms":now.as_millis() as u64,
            "recipients":audit,"health_updates":updates.iter()
                .map(|packet| protocol::encode_hex(packet)).collect::<Vec<_>>()}),
            wall,
        )?;
        for (reservation, sessions, remaining, health, launcher_state) in planned {
            if let Some(actor) = self
                .active
                .as_mut()
                .filter(|actor| actor.reservation.id == reservation)
                .or_else(|| self.peers.get_mut(&reservation))
            {
                actor.static_resupply = sessions;
                for (gear, count) in crate::GRENADE_GEARS.into_iter().zip(remaining) {
                    actor.set_grenade_count(gear, count);
                }
                if let Some(state) = launcher_state {
                    actor.infantry_rocket_state = state;
                }
                if let Some((damage, generation)) = health {
                    actor.last_damage = damage;
                    actor.life = CharacterLife::Alive {
                        generation,
                        health_quarter_points: damage.health_quarter_points,
                    };
                }
            }
        }
        for actor in self
            .active
            .iter_mut()
            .chain(self.peers.values_mut())
            .filter(|actor| actor.initial_objects.is_some())
        {
            for packet in &updates {
                actor.queue_shared(packet.clone());
                actor.ordered_health_pending = true;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spawn_progression::{dispatch, ready};
    use std::sync::Arc;

    fn document() -> Value {
        document_for(PANAMA, 1, 9)
    }

    fn document_for(map: &str, mode: i8, count: u32) -> Value {
        let hash = "a".repeat(64);
        json!({
            "schema_version":1,"format":"mag_retail_human_static_resupply",
            "application_id":20231,"map":map,"mode":mode,"entity_family":42,
            "attacking_faction":0,"spawn_sha256":hash,"collision_sha256":hash,
            "source":{"retail_elf_sha256":hash,"level_archive_sha256":hash,
                "clv_sha256":hash,"clv_member":format!("/run/mag/ps3/levels/{map}/{map}.clv"),
                "id_scope":"retail_fresh_successful_compiled_load","allocation_pool_capacity":32,
                "allocation_proof_sha256":hash,"definition":{"offset":32,"token":99,"sha256":hash}},
            "providers":(0..count).map(|index| json!({
                "packed_target":FAMILY_PREFIX | index,"low16":index,
                "tick_rate_seconds":1.0,"percent_refresh_rate":7.0,
                "box":{"center":[20.0,0.0,0.0],"axes":[[0.0,0.0,1.0],[0.0,1.0,0.0],[-1.0,0.0,0.0]],
                    "euler_xyz_degrees":[0.0,-90.0,0.0],"scales_xyz":[4.0,2.0,6.0],
                    "start_tolerance_m":1.0,"unit_half_extent":0.5},
                "source":{"instance_offset":100+index,"instance_name_token":200+index,
                    "instance_sha256":hash,"parent_instance_sha256":hash,"prefab_path":[300+index,0]}
            })).collect::<Vec<_>>()
        })
    }

    fn parse(value: &Value) -> Result<Catalog, &'static str> {
        Catalog::from_bytes(&serde_json::to_vec(value).unwrap())
    }

    fn fixture() -> Engine<Vec<u8>> {
        let mut engine = acquisition_structural::connected_tests::fixture();
        Arc::get_mut(&mut engine.acquisition_gameplay.as_mut().unwrap().setup)
            .unwrap()
            .human_static_resupply = Some(Arc::new(parse(&document()).unwrap()));
        engine.item_catalog = human_kill_reward_tests::grenade_items_with_cover();
        let mut equipped_engine = human_kill_reward_tests::fixture();
        let equipped = equipped_engine.active.as_mut().unwrap();
        let mut kit = equipped.spawned_loadout.take().unwrap();
        kit.inventories[0].slots[4] = Some(4);
        let source = engine.active.as_mut().unwrap();
        ready(source);
        source.mounted_bunker = None;
        source.mounted_seat = None;
        source.grenade_remaining = 1;
        source.current_inventory_handle = equipped.current_inventory_handle;
        source.spawned_loadout = Some(kit);
        source.life = CharacterLife::Alive {
            generation: 4,
            health_quarter_points: 400,
        };
        source.last_damage.health_quarter_points = 400;
        set_position(&mut engine, 20.0);
        engine
    }

    fn set_position(engine: &mut Engine<Vec<u8>>, x: f32) {
        engine.active.as_mut().unwrap().tracking.server_position(
            replication::CharacterPosition { x, y: 0., z: 0. },
            player_tracking::PositionOrigin::ServerSpawn,
            1,
        );
    }

    fn request(phase: u8, target: u32) -> Vec<u8> {
        let mut bytes = vec![2, 11, phase];
        bytes.extend(target.to_be_bytes());
        bytes
    }

    fn tick(engine: &mut Engine<Vec<u8>>, now_ms: u64) -> Result<(), &'static str> {
        engine.tick_human_static_resupply(Duration::from_millis(now_ms), 1)
    }

    fn ammo(engine: &Engine<Vec<u8>>) -> u8 {
        engine.active.as_ref().unwrap().grenade_remaining
    }

    #[test]
    fn human_static_resupply_refills_configured_smoke_without_frag_metadata() {
        let mut engine = fixture();
        let faction = engine
            .active
            .as_ref()
            .unwrap()
            .reservation
            .character
            .faction;
        let variants = std::array::from_fn(|index| crate::smoke_grenades::Variant {
            faction: index as u8,
            item_id: 451 + index as u16,
            ammo_token: 800 + index as u32,
            initial_count: 2,
            fuse_ms: 100,
            projectile_lifetime_ms: 2000,
            volume_duration_ms: 750,
        });
        engine.smoke_grenades = Some(Arc::new(crate::smoke_grenades::Catalog::synthetic(
            variants,
        )));
        let actor = engine.active.as_mut().unwrap();
        actor.spawned_loadout.as_mut().unwrap().inventories[0].slots[4] =
            Some(451 + u16::from(faction));
        actor.grenade_remaining = 0;
        dispatch(&mut engine, &request(0, FAMILY_PREFIX), 200).unwrap();
        tick(&mut engine, 1199).unwrap();
        assert_eq!(ammo(&engine), 0);
        tick(&mut engine, 1200).unwrap();
        assert_eq!(ammo(&engine), 2);
        for (slot, now) in [(3, 1201), (2, 1202)] {
            let launch = hospital_rocket_runtime::tests::launch_wire(&engine, slot, 4);
            dispatch(&mut engine, &launch, now).unwrap();
        }
        assert_eq!(ammo(&engine), 0);
        assert!(engine.active.as_ref().unwrap().projectiles.get(2).is_some());
        assert!(engine.active.as_ref().unwrap().projectiles.get(3).is_some());
    }

    #[test]
    fn human_static_resupply_refills_configured_flash_to_three_without_skill_53() {
        let mut engine = fixture();
        let variants = std::array::from_fn(|f| crate::flash_grenades::Variant {
            faction: f as u8,
            item_id: 451 + f as u16,
            ammo_token: 800 + f as u32,
            initial_count: 3,
            fuse_ms: 100,
            projectile_lifetime_ms: 2000,
            inner_radius_m: 10.,
            outer_radius_m: 40.,
            material_penetration: 1.7,
        });
        engine.flash_grenades = Some(Arc::new(crate::flash_grenades::Catalog::synthetic(
            variants,
        )));
        let actor = engine.active.as_mut().unwrap();
        actor.spawned_loadout.as_mut().unwrap().inventories[0].slots[4] =
            Some(451 + u16::from(actor.reservation.character.faction));
        actor.grenade_remaining = 0;
        actor.reservation.character.skills = Some([1, 0, 0, 0, 0]);
        dispatch(&mut engine, &request(0, FAMILY_PREFIX), 200).unwrap();
        tick(&mut engine, 1199).unwrap();
        assert_eq!(ammo(&engine), 0);
        tick(&mut engine, 1200).unwrap();
        assert_eq!(ammo(&engine), 3);
        for (slot, now) in [(3, 1201), (2, 1202), (1, 1203)] {
            let wire = hospital_rocket_runtime::tests::launch_wire(&engine, slot, 4);
            dispatch(&mut engine, &wire, now).unwrap();
        }
        assert_eq!(ammo(&engine), 0);
    }

    #[test]
    fn human_static_resupply_native_start_duplicate_due_and_refire() {
        let mut engine = fixture();
        let launch = hospital_rocket_runtime::tests::launch_wire(&engine, 3, 4);
        dispatch(&mut engine, &launch, 100).unwrap();
        assert_eq!(ammo(&engine), 0);
        assert!(engine.active.as_ref().unwrap().projectiles.get(3).is_some());
        let start = request(0, FAMILY_PREFIX);
        assert!(dispatch(&mut engine, &start, 200).unwrap().is_empty());
        assert!(dispatch(&mut engine, &start, 1100).unwrap().is_empty());
        tick(&mut engine, 1199).unwrap();
        assert_eq!(ammo(&engine), 0);
        tick(&mut engine, 1200).unwrap();
        assert_eq!(ammo(&engine), 1);
        let second_launch = hospital_rocket_runtime::tests::launch_wire(&engine, 2, 4);
        dispatch(&mut engine, &second_launch, 1201).unwrap();
        assert_eq!(ammo(&engine), 0);
        assert!(engine.active.as_ref().unwrap().projectiles.get(2).is_some());
        tick(&mut engine, 1200).unwrap();
        assert_eq!(ammo(&engine), 0);
    }

    #[test]
    fn human_static_resupply_rejects_wrong_world_and_plugin() {
        let mut engine = fixture();
        let packet = request(0, FAMILY_PREFIX);
        let actor = engine.active.as_ref().unwrap();
        let decoded = DecodedEvent::with_profile(
            &packet,
            actor.reservation.player,
            engine.wire_profile,
            engine.ground_driver_enabled(),
        );
        let anchor = (
            actor.reservation.id.clone(),
            actor.reservation.player,
            actor.reservation.world,
        );
        let event = |world, plugin| events::EventInput {
            reservation_id: anchor.0.clone(),
            player_id: anchor.1,
            world_id: world,
            transport: Transport::Udp,
            direction: Direction::ClientToServer,
            rt_type: 0x3d,
            plugin_id: Some(plugin),
            payload_hex: protocol::encode_hex(&packet),
        };
        assert_eq!(
            engine
                .handle_event(
                    event(anchor.2 + 1, 1),
                    &decoded,
                    Duration::from_millis(200),
                    1
                )
                .unwrap_err(),
            "identity_mismatch",
        );
        engine
            .handle_event(event(anchor.2, 2), &decoded, Duration::from_millis(300), 1)
            .unwrap();
        assert!(engine.active.as_ref().unwrap().static_resupply.is_empty());
    }

    #[test]
    fn human_static_resupply_stop_cancels_only_the_exact_owned_provider() {
        let mut engine = fixture();
        engine.active.as_mut().unwrap().grenade_remaining = 0;
        let start = request(0, FAMILY_PREFIX);
        dispatch(&mut engine, &start, 200).unwrap();
        dispatch(&mut engine, &request(1, FAMILY_PREFIX | 1), 300).unwrap();
        assert_eq!(engine.active.as_ref().unwrap().static_resupply.len(), 1);
        dispatch(&mut engine, &request(1, FAMILY_PREFIX), 400).unwrap();
        tick(&mut engine, 1400).unwrap();
        assert_eq!(ammo(&engine), 0);
        assert!(engine.active.as_ref().unwrap().static_resupply.is_empty());
    }

    #[test]
    fn human_static_resupply_requires_exact_provider_and_authored_position() {
        let catalog = parse(&document()).unwrap();
        let provider = catalog.provider(FAMILY_PREFIX).unwrap();
        assert!(provider.bounds.contains([24., 2., 3.]));
        assert!(!provider.bounds.contains([24.001, 2., 3.]));
        let mut engine = fixture();
        set_position(&mut engine, 24.01);
        dispatch(&mut engine, &request(0, FAMILY_PREFIX), 200).unwrap();
        set_position(&mut engine, 20.);
        dispatch(&mut engine, &request(0, FAMILY_PREFIX | 0xffff), 300).unwrap();
        assert!(engine.active.as_ref().unwrap().static_resupply.is_empty());

        dispatch(&mut engine, &request(0, FAMILY_PREFIX), 400).unwrap();
        set_position(&mut engine, 25.);
        tick(&mut engine, 1400).unwrap();
        assert!(engine.active.as_ref().unwrap().static_resupply.is_empty());
    }

    #[test]
    fn human_static_resupply_refused_record_preserves_timer_and_budget() {
        let mut engine = fixture();
        engine.active.as_mut().unwrap().grenade_remaining = 0;
        dispatch(&mut engine, &request(0, FAMILY_PREFIX), 200).unwrap();
        engine.max_log = 0;
        assert!(tick(&mut engine, 1200).is_err());
        assert_eq!(ammo(&engine), 0);
        let session = &engine.active.as_ref().unwrap().static_resupply[0];
        assert_eq!(session.updated, Duration::from_millis(200));
        assert_eq!(session.elapsed, 0.0);
    }

    #[test]
    fn human_static_resupply_refused_start_does_not_install_a_timer() {
        let mut engine = fixture();
        engine.max_log = engine.written;
        assert!(dispatch(&mut engine, &request(0, FAMILY_PREFIX), 200).is_err());
        assert!(engine.active.as_ref().unwrap().static_resupply.is_empty());
    }

    #[test]
    fn human_static_resupply_mount_cancels_before_a_due_pulse() {
        let mut engine = fixture();
        engine.active.as_mut().unwrap().grenade_remaining = 0;
        dispatch(&mut engine, &request(0, FAMILY_PREFIX), 200).unwrap();
        engine.active.as_mut().unwrap().mounted_seat = Some(1);
        tick(&mut engine, 1200).unwrap();
        assert_eq!(ammo(&engine), 0);
        assert!(engine.active.as_ref().unwrap().static_resupply.is_empty());
    }

    #[test]
    fn human_static_resupply_refused_staged_lethal_preserves_live_session() {
        use crate::acquisition_character_effects::{NativeState, Request, Target};
        let mut engine = fixture();
        dispatch(&mut engine, &request(0, FAMILY_PREFIX), 200).unwrap();
        let actor = engine.active.as_ref().unwrap();
        assert_eq!(actor.static_resupply.len(), 1);
        let target = Target {
            character_id: 0x0380_0000 | u32::from(actor.reservation.player),
            generation: 4,
            world: actor.reservation.world,
        };
        let mut native = std::collections::BTreeMap::from([(
            target.character_id,
            NativeState {
                invulnerability: Some(2),
                associated_vehicle: Some(protocol::VictimWasMounted::NoAssociatedVehicle),
            },
        )]);
        let candidate = engine
            .prepare_acquisition_character_effects(
                &[(
                    target,
                    Request::VolumeLethal {
                        victim_vector: [1., 0., 0.],
                    },
                )],
                &native,
                Duration::from_millis(250),
            )
            .unwrap();
        assert!(engine.active.as_ref().unwrap().life.is_alive());
        engine.max_log = engine.written;
        assert!(
            engine
                .record_acquisition_character_effects(candidate, &mut native, 1)
                .is_err()
        );
        let actor = engine.active.as_ref().unwrap();
        assert!(actor.life.is_alive());
        assert_eq!(actor.static_resupply.len(), 1);
    }

    #[test]
    fn human_static_resupply_late_update_consumes_only_one_interval() {
        let mut engine = fixture();
        engine.active.as_mut().unwrap().grenade_remaining = 0;
        dispatch(&mut engine, &request(0, FAMILY_PREFIX), 200).unwrap();
        tick(&mut engine, 3700).unwrap();
        assert_eq!(ammo(&engine), 1);
        let session = &engine.active.as_ref().unwrap().static_resupply[0];
        assert_eq!(session.elapsed, 2.5);
        assert_eq!(session.updated, Duration::from_millis(3700));
    }

    #[test]
    fn human_static_resupply_due_uses_stored_interval_before_refreshing_modifier() {
        let qualified_modifiers = |modifier| {
            let catalog = skill_catalog::SkillCatalog::from_value(json!({
                "application_id":20231,"schema_version":2,
                "table":{"Skills":{"MagCharacterSkill0":{"MagCharacterSkill":{
                    "Default":{"Modifier":"kCharModResupplyRate","ModifierValue":modifier},
                    "Green":{"Available":true},"Black":{"Available":true},"Red":{"Available":true}
                }}}}
            }))
            .unwrap();
            catalog.resolve(0, &[1, 0, 0, 0, 0]).unwrap()
        };
        let mut engine = fixture();
        let actor = engine.active.as_mut().unwrap();
        actor.modifiers = qualified_modifiers(0.25);
        actor.grenade_remaining = 0;
        dispatch(&mut engine, &request(0, FAMILY_PREFIX), 200).unwrap();
        let actor = engine.active.as_mut().unwrap();
        assert_eq!(actor.static_resupply[0].interval, 1.25);
        actor.modifiers = qualified_modifiers(0.5);
        tick(&mut engine, 1449).unwrap();
        assert_eq!(ammo(&engine), 0);
        tick(&mut engine, 1450).unwrap();
        assert_eq!(ammo(&engine), 1);
        let session = &engine.active.as_ref().unwrap().static_resupply[0];
        assert_eq!(session.interval, 1.5);
        assert_eq!(session.elapsed, 0.0);
    }

    #[test]
    fn human_static_resupply_requires_current_life_and_round() {
        let mut engine = fixture();
        let start = request(0, FAMILY_PREFIX);
        engine.active.as_mut().unwrap().grenade_remaining = 0;
        dispatch(&mut engine, &start, 200).unwrap();
        engine.active.as_mut().unwrap().life = CharacterLife::Alive {
            generation: 5,
            health_quarter_points: 400,
        };
        tick(&mut engine, 1200).unwrap();
        assert_eq!(ammo(&engine), 0);
        assert!(engine.active.as_ref().unwrap().static_resupply.is_empty());
        dispatch(&mut engine, &start, 1300).unwrap();
        engine.round_number += 1;
        tick(&mut engine, 2300).unwrap();
        assert_eq!(ammo(&engine), 0);
        assert!(engine.active.as_ref().unwrap().static_resupply.is_empty());
    }

    #[test]
    fn human_static_resupply_catalog_checks_each_mode_map_inventory() {
        for (map, mode, count) in [
            ("mag_alaska_subpen_final", 1, 10),
            (PANAMA, 1, 9),
            ("mag_voz_final", 1, 11),
            ("mag_baku_final", 2, 20),
            ("mag_canal_final", 2, 20),
            ("mag_unalaska_final", 2, 21),
        ] {
            assert!(parse(&document_for(map, mode, count)).is_ok());
            assert!(parse(&document_for(map, 3 - mode, count)).is_err());
            assert!(parse(&document_for(map, mode, count - 1)).is_err());
        }
    }

    #[test]
    fn human_static_resupply_catalog_rejects_unqualified_id_and_shape() {
        let mut invalid = document();
        invalid["providers"][0]["packed_target"] = json!(FAMILY_PREFIX | 1);
        assert!(parse(&invalid).is_err());
        let mut invalid = document();
        invalid["providers"][0]["box"]["axes"][0] = json!([0., 1., 0.]);
        assert!(parse(&invalid).is_err());
        let mut invalid = document();
        invalid["source"]["unexpected"] = json!(true);
        assert!(parse(&invalid).is_err());
    }
    fn wounded(engine: &mut Engine<Vec<u8>>, hp: u16) {
        let actor = engine.active.as_mut().unwrap();
        actor.life = CharacterLife::Alive {
            generation: 4,
            health_quarter_points: hp,
        };
        actor.last_damage = replication::CharacterDamage {
            health_quarter_points: hp,
            last_location_and_type: 0x23,
            conditions: 4,
            last_attacker: 0x0380_0007,
            last_source_object: 0x0680_0002,
            last_user_data: 123,
        };
        actor.grenade_remaining = 0;
    }

    fn configure_health_rate(engine: &mut Engine<Vec<u8>>, rate: f32) {
        let mut catalog = document();
        for provider in catalog["providers"].as_array_mut().unwrap() {
            provider["percent_refresh_rate"] = json!(rate);
        }
        Arc::get_mut(&mut engine.acquisition_gameplay.as_mut().unwrap().setup)
            .unwrap()
            .human_static_resupply = Some(Arc::new(parse(&catalog).unwrap()));
    }

    fn health(engine: &Engine<Vec<u8>>) -> u16 {
        engine
            .active
            .as_ref()
            .unwrap()
            .last_damage
            .health_quarter_points
    }

    #[test]
    fn human_static_resupply_health_without_frag_and_exact_timer() {
        let mut engine = fixture();
        configure_health_rate(&mut engine, 10.);
        wounded(&mut engine, 250);
        let actor = engine.active.as_mut().unwrap();
        actor.spawned_loadout.as_mut().unwrap().inventories[0].slots[4] = None;
        let skills = skill_catalog::SkillCatalog::from_value(json!({
            "schema_version":2,"application_id":20231,"table":{"Skills":{"MagCharacterSkill0":{"MagCharacterSkill":{
            "Default":{"Modifier":"kCharModHealth","ModifierValue":0.2},"Green":{"Available":true}}}}}
        })).unwrap();
        actor.modifiers = skills.resolve(0, &[1, 0, 0, 0, 0]).unwrap();
        dispatch(&mut engine, &request(0, FAMILY_PREFIX), 200).unwrap();
        tick(&mut engine, 1199).unwrap();
        assert_eq!(health(&engine), 250);
        let before_frames = engine.active.as_ref().unwrap().ordinary_shared_len();
        tick(&mut engine, 1200).unwrap();
        assert_eq!(health(&engine), 298);
        let actor = engine.active.as_ref().unwrap();
        let model = actor.last_damage;
        assert_eq!(
            (
                model.last_location_and_type,
                model.conditions,
                model.last_attacker,
                model.last_source_object,
                model.last_user_data
            ),
            (0x63, 4, 0x0380_0007, 0x0680_0002, 123)
        );
        assert!(matches!(
            actor.life,
            CharacterLife::Alive {
                generation: 4,
                health_quarter_points: 298
            }
        ));
        assert_eq!(actor.ordinary_shared_len(), before_frames + 1);
        assert!(actor.ordered_health_pending);
        assert!(actor.reward_statistics.is_empty());
        assert!(engine.pending_rewards.is_empty());
        assert_eq!(ammo(&engine), 0);
        dispatch(&mut engine, &request(1, FAMILY_PREFIX), 1250).unwrap();
        tick(&mut engine, 2200).unwrap();
        assert_eq!(health(&engine), 298);
    }

    #[test]
    fn human_static_resupply_health_combines_pulses_into_one_frame_and_caps() {
        let mut engine = fixture();
        configure_health_rate(&mut engine, 10.);
        wounded(&mut engine, 250);
        for target in [FAMILY_PREFIX, FAMILY_PREFIX | 1] {
            dispatch(&mut engine, &request(0, target), 200).unwrap();
        }
        let baseline = engine.active.as_ref().unwrap().ordinary_shared_len();
        tick(&mut engine, 1200).unwrap();
        assert_eq!(health(&engine), 330);
        assert_eq!(ammo(&engine), 1);
        assert_eq!(
            engine.active.as_ref().unwrap().ordinary_shared_len(),
            baseline + 1
        );
        wounded(&mut engine, 396);
        tick(&mut engine, 2200).unwrap();
        assert_eq!(health(&engine), 400);
        assert_eq!(
            engine.active.as_ref().unwrap().ordinary_shared_len(),
            baseline + 2
        );
        tick(&mut engine, 3200).unwrap();
        assert_eq!(
            engine.active.as_ref().unwrap().ordinary_shared_len(),
            baseline + 2
        );
    }

    #[test]
    fn human_static_resupply_health_defers_without_consuming_due_pulse() {
        let mut engine = fixture();
        configure_health_rate(&mut engine, 10.);
        wounded(&mut engine, 250);
        dispatch(&mut engine, &request(0, FAMILY_PREFIX), 200).unwrap();
        engine.enable_world_ticks();
        let batches = engine.ground_vehicles.as_ref().unwrap().batch_count();
        engine.active.as_mut().unwrap().ground_vehicle_batch = batches;
        let prime = engine.tick_world(Duration::from_millis(250), 1, "prime".into());
        assert!(prime.ok, "{:?}", prime.error);
        let filler = replication::CharacterDamageUpdate {
            sequence: 2,
            timestamp_ms: 1000,
            object_index: engine.active.as_ref().unwrap().reservation.player,
            damage: engine.active.as_ref().unwrap().last_damage,
            forced_state: None,
        }
        .encode_with_profile(engine.wire_profile)
        .unwrap();
        // Current valid model packets occupy the ordinary FIFO; the reserved
        // Hospital bootstrap at its tail does not consume ordinary capacity.
        let actor = engine.active.as_mut().unwrap();
        actor
            .pending_shared
            .resize(multiplayer::MAX_SHARED_QUEUE, filler.clone());
        actor.queue_hospital_bootstrap(vec![filler]);
        let due = engine.tick_world(Duration::from_millis(1200), 1, "due".into());
        assert!(due.ok, "{:?}", due.error);
        assert!(!due.messages.is_empty(), "queue must still drain");
        assert_eq!(health(&engine), 250);
        assert_eq!(ammo(&engine), 0);
        let actor = engine.active.as_ref().unwrap();
        assert!(!actor.replication_backlogged);
        assert_eq!(actor.static_resupply[0].updated, Duration::from_millis(200));
        assert!(actor.ordinary_shared_len() < multiplayer::MAX_SHARED_QUEUE);
        let retry = engine.tick_world(Duration::from_millis(1250), 1, "retry".into());
        assert!(retry.ok, "{:?}", retry.error);
        assert_eq!(health(&engine), 290);
        assert_eq!(ammo(&engine), 1);
    }

    #[test]
    fn human_static_resupply_health_recorder_failure_preserves_candidate_state() {
        let mut engine = fixture();
        configure_health_rate(&mut engine, 10.);
        wounded(&mut engine, 250);
        dispatch(&mut engine, &request(0, FAMILY_PREFIX), 200).unwrap();
        let baseline = engine.active.as_ref().unwrap().ordinary_shared_len();
        engine.max_log = engine.written;
        assert_eq!(tick(&mut engine, 1200).unwrap_err(), "log_limit_reached");
        let actor = engine.active.as_ref().unwrap();
        assert_eq!(actor.static_resupply[0].updated, Duration::from_millis(200));
        assert_eq!(actor.ordinary_shared_len(), baseline);
        assert!(!actor.ordered_health_pending);
        assert_eq!(health(&engine), 250);
        assert_eq!(ammo(&engine), 0);
    }
    #[test]
    fn human_static_resupply_refills_spent_launcher_with_record_and_stop() {
        use crate::infantry_rocket_runtime::tests as rockets;
        for refuse_record in [false, true] {
            let mut engine = rockets::fixture();
            let mut document = document_for("mag_baku_final", 2, 20);
            for provider in document["providers"].as_array_mut().unwrap() {
                provider["percent_refresh_rate"] = json!(0.0);
            }
            engine
                .domination_gameplay
                .as_mut()
                .unwrap()
                .state
                .human_static_resupply = Some(Arc::new(parse(&document).unwrap()));
            let actor = engine.active.as_mut().unwrap();
            let kit = actor.spawned_loadout.as_mut().unwrap();
            kit.inventories[usize::from(kit.handle.inventory_index)].slots[4] = None;
            set_position(&mut engine, 20.0);
            rockets::launch(&mut engine, 0, 64, 1000);
            assert_eq!(rockets::rounds(&engine), 3);
            dispatch(&mut engine, &request(0, FAMILY_PREFIX), 1100).unwrap();
            assert_eq!(engine.active.as_ref().unwrap().static_resupply.len(), 1);
            tick(&mut engine, 2099).unwrap();
            assert_eq!(rockets::rounds(&engine), 3);
            let written = engine.written;
            if refuse_record {
                engine.max_log = written;
                assert_eq!(tick(&mut engine, 2100).unwrap_err(), "log_limit_reached");
                assert_eq!(rockets::rounds(&engine), 3);
                assert_eq!(engine.written, written);
                assert_eq!(
                    engine.active.as_ref().unwrap().static_resupply[0].updated,
                    Duration::from_millis(1100)
                );
            } else {
                tick(&mut engine, 2100).unwrap();
                assert_eq!(rockets::rounds(&engine), 4);
                assert!(
                    engine
                        .active
                        .as_ref()
                        .unwrap()
                        .infantry_rocket_state
                        .occupies(0)
                );
                rockets::launch(&mut engine, 1, 65, 2200);
                assert_eq!(rockets::rounds(&engine), 3);
                dispatch(&mut engine, &request(1, FAMILY_PREFIX), 2300).unwrap();
                tick(&mut engine, 3100).unwrap();
                assert_eq!(rockets::rounds(&engine), 3);
            }
        }
    }
}
