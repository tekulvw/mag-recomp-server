//! Explicit retail static supply bindings for ordinary human grenade refill.
use super::*;
use serde::Deserialize;
use std::{collections::BTreeSet, path::Path};

const MAX_BYTES: u64 = 65_536;
const FAMILY_PREFIX: u32 = 0x2a80_0000;
const PANAMA: &str = "mag_panama_military_base_final";

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
            || catalog.map != PANAMA
            || catalog.mode != 1
            || catalog.entity_family != 42
            || ![0, 2].contains(&catalog.attacking_faction)
            || source.id_scope != "retail_fresh_successful_compiled_load"
            || source.allocation_pool_capacity != 32
            || source.clv_member != format!("/run/mag/ps3/levels/{PANAMA}/{PANAMA}.clv")
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
            || catalog.providers.len() != 9
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

fn equipped_frag(actor: &Active, items: &item_catalog::ItemCatalog) -> Option<(u16, u8)> {
    let kit = actor.equipped_loadout()?;
    let item = kit
        .inventories
        .get(usize::from(kit.handle.inventory_index))?
        .slots[4]?;
    // This scope only admits the selected ordinary retail frag via the same
    // slot and catalog that authorize its normal launch.
    let cap = items.grenade(item).ok()?.initial_count;
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
            .as_ref()?
            .setup
            .human_static_resupply
            .as_deref()
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
        if equipped_frag(actor, &self.item_catalog).is_none() {
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
        let mut audit = Vec::new();
        for actor in self.active.iter().chain(self.peers.values()) {
            if actor.static_resupply.is_empty() {
                continue;
            }
            let generation = living_generation(actor);
            let frag = equipped_frag(actor, &self.item_catalog);
            let mut sessions = Vec::with_capacity(actor.static_resupply.len());
            let mut remaining = actor.grenade_remaining;
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
                let Some((item, cap)) = frag else {
                    changed = true;
                    audit.push(json!({"reservation":actor.reservation.id,"provider":session.target,"operation":"cancel"}));
                    continue;
                };
                let elapsed = session.elapsed + now.saturating_sub(session.updated).as_secs_f32();
                if elapsed < session.interval || now <= session.updated {
                    sessions.push(*session);
                    continue;
                }
                let mut next = *session;
                next.elapsed = elapsed - session.interval;
                next.interval = interval(
                    provider.tick_rate_seconds,
                    leadership.modifiers(actor).value("kCharModResupplyRate"),
                )?;
                next.updated = now;
                audit.push(json!({"reservation":actor.reservation.id,"provider":session.target,"operation":"pulse",
                    "world":actor.reservation.world,"round":session.round,"generation":session.generation,
                    "item":item,"before":remaining,"after":cap,"interval_seconds":next.interval}));
                remaining = cap;
                sessions.push(next);
                changed = true;
            }
            if changed {
                planned.push((actor.reservation.id.clone(), sessions, remaining));
            }
        }
        if planned.is_empty() {
            return Ok(());
        }
        self.record(
            json!({"op":"human_static_resupply_tick","timestamp_ms":now.as_millis() as u64,
            "recipients":audit}),
            wall,
        )?;
        for (reservation, sessions, remaining) in planned {
            if let Some(actor) = self
                .active
                .as_mut()
                .filter(|actor| actor.reservation.id == reservation)
                .or_else(|| self.peers.get_mut(&reservation))
            {
                actor.static_resupply = sessions;
                actor.grenade_remaining = remaining;
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
        let hash = "a".repeat(64);
        json!({
            "schema_version":1,"format":"mag_retail_human_static_resupply",
            "application_id":20231,"map":PANAMA,"mode":1,"entity_family":42,
            "attacking_faction":0,"spawn_sha256":hash,"collision_sha256":hash,
            "source":{"retail_elf_sha256":hash,"level_archive_sha256":hash,
                "clv_sha256":hash,"clv_member":format!("/run/mag/ps3/levels/{PANAMA}/{PANAMA}.clv"),
                "id_scope":"retail_fresh_successful_compiled_load","allocation_pool_capacity":32,
                "allocation_proof_sha256":hash,"definition":{"offset":32,"token":99,"sha256":hash}},
            "providers":(0..9).map(|index| json!({
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
}
