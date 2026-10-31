//! Match-owned emplacement bodies. Placement and combat data load from disk;
//! request identity is bound to Horizon admission, never the repair wire.
use super::*;
use replication::{
    BitWriter, DefensiveEmplacementCreation, UpdateHeader, VehicleDamage, VehicleState,
};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::Path,
};
use structure_catalog::{StructureCatalog, StructureProfile};

#[derive(Clone, Copy, Debug)]
pub struct BunkerHealth {
    hp: f32,
    max: f32,
    destroyed: bool,
    rules: structure_catalog::NativeStructureRules,
    rebuild_percent: f32,
}
impl BunkerHealth {
    pub fn new(max: f32) -> Result<Self, &'static str> {
        if !max.is_finite() || !(1.0..=1023.75).contains(&max) {
            return Err("invalid_bunker_health");
        }
        Ok(Self {
            hp: max,
            max,
            destroyed: false,
            rules: structure_catalog::NativeStructureRules {
                light_percent: 70.0,
                heavy_percent: 40.0,
                critical_percent: 15.0,
                dead_health_floor: 1.0,
                min_rebuild_percent: 15.0,
            },
            rebuild_percent: 100.0,
        })
    }
    fn for_profile(
        profile: &StructureProfile,
        catalog: &StructureCatalog,
    ) -> Result<Self, &'static str> {
        let mut health = Self::new(profile.max_health)?;
        health.rules = catalog.rules();
        health.rebuild_percent = profile
            .repaired_health_percent
            .max(health.rules.min_rebuild_percent);
        Ok(health)
    }
    pub fn hp(self) -> f32 {
        self.hp
    }
    pub fn wire_health(self) -> u16 {
        (self.hp * 4.0).trunc() as u16
    }
    fn set(&mut self, hp: f32) {
        self.hp = if hp < self.rules.dead_health_floor {
            0.0
        } else {
            hp.min(self.max)
        };
    }
    pub fn damage(&mut self, amount: f32) -> Result<(), &'static str> {
        if !amount.is_finite() || amount < 0.0 {
            return Err("invalid_bunker_damage");
        }
        self.set((self.hp - amount).max(0.0));
        if self.hp == 0.0 {
            self.destroyed = true;
        }
        Ok(())
    }
    pub fn repair(&mut self, amount: f32) -> Result<(), &'static str> {
        if !amount.is_finite() || amount < 0.0 {
            return Err("invalid_bunker_repair");
        }
        self.set(self.hp + amount);
        if self.hp >= self.max * (self.rebuild_percent / 100.0) {
            self.destroyed = false;
        }
        Ok(())
    }
    pub fn conditions(self) -> u8 {
        // Native condition flags are independent health threshold bits.
        u8::from(self.hp < self.max * (self.rules.light_percent / 100.0))
            | (u8::from(self.hp < self.max * (self.rules.heavy_percent / 100.0)) << 1)
            | (u8::from(self.hp < self.max * (self.rules.critical_percent / 100.0)) << 2)
            | (u8::from(self.destroyed) << 3)
    }
}
#[derive(Deserialize)]
struct Catalog {
    format: String,
    schema_version: u8,
    vehicle_type_token: u32,
    maps: Vec<Map>,
    faction_definitions: Vec<Faction>,
}
#[derive(Deserialize)]
struct Faction {
    faction_id: u8,
    mount_points_for_comparison: Vec<Mount>,
}
#[derive(Deserialize)]
struct Mount {
    local_offset_xyz: [f32; 3],
}
#[derive(Deserialize)]
struct Map {
    name: String,
    runtime_indices_verified: bool,
    bunkers: Vec<Placement>,
}
#[derive(Deserialize)]
struct Placement {
    catalog_id: String,
    team: u8,
    active: bool,
    spawn_catalog_ordinal: u16,
    spawn_runtime_index: Option<u16>,
    calculated_vehicle_origin_xyz: [f32; 3],
    quaternion_xyzw: [f32; 4],
    vehicle_properties_source: Source,
}
#[derive(Deserialize)]
struct Source {
    properties_zip_member: String,
}
#[derive(Clone)]
struct Bunker {
    id: String,
    spawn: u16,
    faction: u8,
    team: u8,
    last_health_update: Duration,
    repair_points: Vec<[f32; 3]>,
    profile: StructureProfile,
    health: BunkerHealth,
    damage: VehicleDamage,
}
pub struct BunkerWorld {
    catalog: StructureCatalog,
    bunkers: BTreeMap<u16, Bunker>,
    vehicle_token: u32,
    pending_placements: usize,
    commands: BTreeMap<String, (u16, f32)>,
}
fn invalid(e: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}
impl BunkerWorld {
    pub fn load(
        placements: impl AsRef<Path>,
        structures: impl AsRef<Path>,
        map_name: &str,
    ) -> io::Result<Self> {
        let raw: Catalog =
            serde_json::from_reader(std::fs::File::open(placements)?).map_err(invalid)?;
        if raw.format != "mag_bunker_catalog" || raw.schema_version != 1 {
            return Err(invalid("invalid bunker catalog"));
        }
        let vehicle_token = raw.vehicle_type_token;
        let catalog = StructureCatalog::load(structures)?;
        let map = raw
            .maps
            .into_iter()
            .find(|m| m.name == map_name)
            .ok_or_else(|| invalid("missing bunker map"))?;
        let mut bunkers = BTreeMap::new();
        let mut spawns = BTreeSet::new();
        let mut pending_placements = 0;
        for p in map.bunkers.into_iter().filter(|p| p.active) {
            let Some(spawn) = p.spawn_runtime_index.or(map
                .runtime_indices_verified
                .then_some(p.spawn_catalog_ordinal))
            else {
                pending_placements += 1;
                continue;
            };
            if spawn >= 128
                || !spawns.insert(spawn)
                || p.team > 1
                || bunkers.len() >= 16
                || p.calculated_vehicle_origin_xyz
                    .iter()
                    .chain(&p.quaternion_xyzw)
                    .any(|x| !x.is_finite())
            {
                return Err(invalid("invalid bunker placement"));
            }
            let source = p
                .vehicle_properties_source
                .properties_zip_member
                .split("/scripts/")
                .nth(1)
                .ok_or_else(|| invalid("invalid property path"))?;
            let profile = catalog.profile_for_source(source).map_err(invalid)?.clone();
            let mounts = &raw
                .faction_definitions
                .iter()
                .find(|f| f.faction_id == profile.faction)
                .ok_or_else(|| invalid("missing faction mounts"))?
                .mount_points_for_comparison;
            let transform = vehicle_exit::BunkerTransform {
                position: replication::CharacterPosition {
                    x: p.calculated_vehicle_origin_xyz[0],
                    y: p.calculated_vehicle_origin_xyz[1],
                    z: p.calculated_vehicle_origin_xyz[2],
                },
                rotation_xyzw: p.quaternion_xyzw,
            };
            let repair_points = mounts
                .iter()
                .map(|m| {
                    let d = transform.rotate(m.local_offset_xyz);
                    [
                        transform.position.x + d[0],
                        transform.position.y + d[1],
                        transform.position.z + d[2],
                    ]
                })
                .collect();
            let health = BunkerHealth::for_profile(&profile, &catalog).map_err(invalid)?;
            bunkers.insert(
                bunkers.len() as u16,
                Bunker {
                    id: p.catalog_id,
                    spawn,
                    faction: profile.faction,
                    team: p.team,
                    last_health_update: Duration::ZERO,
                    repair_points,
                    profile,
                    health,
                    damage: VehicleDamage {
                        last_location_and_type: 0,
                        health_quarter_points: health.wire_health(),
                        conditions: 0,
                        last_attacker: u32::MAX,
                        last_source_object: u32::MAX,
                        last_user_data: 0,
                    },
                },
            );
        }
        if bunkers.is_empty() {
            return Err(invalid("no verified active bunker placements"));
        }
        Ok(Self {
            catalog,
            bunkers,
            vehicle_token,
            pending_placements,
            commands: Default::default(),
        })
    }
    /// A map with no defensive emplacements: combat rules are still loaded from
    /// the structure catalog, but no placements or bunker spawns exist.
    pub fn without_bunkers(structures: impl AsRef<Path>) -> io::Result<Self> {
        Ok(Self {
            catalog: StructureCatalog::load(structures)?,
            bunkers: BTreeMap::new(),
            vehicle_token: 0,
            pending_placements: 0,
            commands: Default::default(),
        })
    }
    pub fn load_default() -> io::Result<Self> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
        Self::load(
            root.join("prototype-bunkers.json"),
            root.join("prototype-structures.json"),
            ACTIVE_MAP_NAME,
        )
    }
    pub fn active_count(&self) -> usize {
        self.bunkers.len()
    }
    pub fn unresolved_count(&self) -> usize {
        self.pending_placements
    }
    pub fn reset(&mut self) {
        for b in self.bunkers.values_mut() {
            b.health = BunkerHealth::for_profile(&b.profile, &self.catalog).unwrap();
            b.damage.health_quarter_points = b.health.wire_health();
            b.damage.conditions = 0;
            b.damage.last_attacker = u32::MAX;
            b.damage.last_source_object = u32::MAX;
            b.damage.last_user_data = 0;
        }
        self.commands.clear();
    }
    fn index(&self, id: u32) -> Option<u16> {
        if id & 0xff7f0000 != 0x0d000000 {
            return None;
        }
        let index = id as u16;
        self.bunkers.contains_key(&index).then_some(index)
    }
    pub(super) fn creations(&self, sequence: u32, time: u32) -> Vec<DefensiveEmplacementCreation> {
        self.bunkers
            .iter()
            .map(|(&index, b)| DefensiveEmplacementCreation {
                sequence,
                timestamp_ms: time,
                object_index: index,
                owner_index: 0x105,
                vehicle_type_token: self.vehicle_token,
                faction: b.faction,
                modifier: 0,
                state: VehicleState::Operational,
                turret_owners: [u16::MAX; 2],
                damage: b.damage,
                team: b.team,
                spawn_point_index: b.spawn,
            })
            .collect()
    }
    fn damage_amount(
        &self,
        index: u16,
        raw: f32,
        pierce: f32,
        redux: f32,
        location: u8,
    ) -> Result<f32, &'static str> {
        let b = self.bunkers.get(&index).ok_or("unknown_bunker")?;
        let armor = self.catalog.armor(b.profile.armor_type)?;
        let body = self.catalog.body(location)?;
        let rating = (armor.rating - pierce).max(0.0);
        let after = (raw - rating).max(0.0);
        let prevention = (armor.prevention - redux).max(0.0);
        let product = armor.coverage[body.armor_area].max(0.0) * prevention;
        Ok((after * (body.multiplier * (1.0 - product))).max(0.0))
    }
}
#[derive(Clone, Copy)]
struct BunkerDelta {
    index: u16,
    damage: VehicleDamage,
}
impl BunkerDelta {
    fn encode_with_profile(
        self,
        seq: u32,
        time: u32,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        Self::encode_list_with_profile(&[self], seq, time, profile)
    }
    fn encode_list_with_profile(
        deltas: &[Self],
        seq: u32,
        time: u32,
        profile: crate::wire_profile::WireProfile,
    ) -> Result<Vec<u8>, &'static str> {
        let mut bits = BitWriter::with_profile(profile);
        UpdateHeader {
            sequence: seq,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 1 << 13,
            timestamp_ms: time,
        }
        .encode(&mut bits)?;
        for delta in deltas {
            bits.write(1, 1)?;
            bits.write(delta.index.into(), 16)?;
            for field in 0..25 {
                bits.write(u32::from(field == 18), 1)?;
                if field == 18 {
                    delta.damage.encode(&mut bits)?;
                }
            }
        }
        bits.write(0, 1)?;
        Ok(bits.finish())
    }
}
impl<W: Write> Engine<W> {
    /// Durability has priority over a periodic clock reply on valid owner updates.
    /// This query does not advance health or consume queued commands.
    pub(super) fn bunker_update_due(&self, now: Duration) -> bool {
        self.active
            .as_ref()
            .is_some_and(|a| a.entered_game && a.ready_for_updates)
            && (!self.pending_bunker_damage.is_empty()
                || self.bunker_world.as_ref().is_some_and(|world| {
                    world.bunkers.values().any(|b| {
                        !b.health.destroyed
                            && b.health.conditions() & 4 != 0
                            && now > b.last_health_update
                    })
                }))
    }
    pub(super) fn tick_bunkers(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<Option<Vec<OutboundMessage>>, &'static str> {
        let Some(actor) = self
            .active
            .as_ref()
            .filter(|a| a.entered_game && a.ready_for_updates)
        else {
            return Ok(None);
        };
        let Some(world) = self.bunker_world.as_ref() else {
            return Ok(None);
        };
        let mut changes = Vec::new();
        for (&index, bunker) in &world.bunkers {
            if bunker.health.destroyed
                || bunker.health.conditions() & 4 == 0
                || now <= bunker.last_health_update
            {
                continue;
            }
            let mut next = bunker.clone();
            let per_second = (bunker.profile.critical_loss_percent_per_second * 0.01)
                * bunker.profile.max_health;
            let amount = per_second * now.saturating_sub(bunker.last_health_update).as_secs_f32();
            next.health.damage(amount)?;
            next.last_health_update = now;
            next.damage.health_quarter_points = next.health.wire_health();
            next.damage.conditions = next.health.conditions();
            changes.push((index, next, amount));
        }
        if changes.is_empty() {
            return Ok(None);
        }
        let deltas: Vec<_> = changes
            .iter()
            .map(|(index, b, _)| BunkerDelta {
                index: *index,
                damage: b.damage,
            })
            .collect();
        let packet = BunkerDelta::encode_list_with_profile(
            &deltas,
            actor.next_sequence()?,
            now.as_millis() as u32,
            self.wire_profile,
        )?;
        let recipient = (
            actor.reservation.id.clone(),
            actor.reservation.world,
            actor.reservation.player,
        );
        self.record(json!({"op":"bunker_critical_decay","changes":changes.iter().map(|(i,b,a)|json!({"object_index":i,"amount_hp":a,"health_hp":b.health.hp(),"conditions":b.damage.conditions})).collect::<Vec<_>>()}),wall)?;
        self.active.as_mut().unwrap().remember(packet.clone())?;
        for (index, next, _) in changes {
            self.bunker_world
                .as_mut()
                .unwrap()
                .bunkers
                .insert(index, next);
        }
        Ok(Some(vec![OutboundMessage {
            reservation_id: recipient.0,
            world_id: recipient.1,
            player_id: recipient.2,
            plugin_id: 1,
            payload_hex: protocol::encode_hex(&packet),
        }]))
    }
    pub fn with_bunker_world(mut self, world: BunkerWorld) -> Self {
        self.bunker_world = Some(world);
        self
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn queue_development_bunker_damage(
        &mut self,
        id: &str,
        world: u32,
        player: u16,
        command: &str,
        target: u32,
        amount: f32,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        if !self.development_control {
            return Err("development_control_disabled");
        }
        let actor = self.active.as_ref().ok_or("no_reservation")?;
        if actor.reservation.id != id
            || actor.reservation.world != world
            || actor.reservation.player != player
        {
            return Err("identity_mismatch");
        }
        if !actor.ready_for_updates || !actor.entered_game {
            return Err("structure_damage_requires_ready_player");
        }
        let w = self
            .bunker_world
            .as_ref()
            .ok_or("bunker_world_unavailable")?;
        let index = w.index(target).ok_or("unknown_bunker")?;
        if let Some(prior) = w.commands.get(command) {
            return if *prior == (index, amount) {
                Ok(Vec::new())
            } else {
                Err("structure_damage_id_conflict")
            };
        }
        if w.commands.len() >= 64 || self.pending_bunker_damage.len() >= 16 {
            return Err("structure_damage_queue_full");
        }
        self.record(json!({"op":"development_structure_damage_queued","reservation_id":id,"target_object_id":target,"damage_id":command,"amount_hp":amount}),wall)?;
        self.bunker_world
            .as_mut()
            .unwrap()
            .commands
            .insert(command.into(), (index, amount));
        self.pending_bunker_damage
            .push_back((index, amount, command.into()));
        Ok(Vec::new())
    }
    pub(super) fn structure_event(
        &mut self,
        message: &Result<protocol::GameMessage<'_>, &'static str>,
        now: Duration,
        wall: u64,
    ) -> Result<Option<Vec<OutboundMessage>>, &'static str> {
        let Ok(message) = message else {
            return Ok(None);
        };
        let active = self.active.as_ref().unwrap();
        let w = self.bunker_world.as_ref();
        let repair = matches!(message, protocol::GameMessage::Repair(_));
        let mut pending = false;
        let critical = false;
        let mut raw = None;
        let mut repair_mount_distance = None;
        let (target, amount, location, userdata, kind) = match message {
            protocol::GameMessage::Repair(r) => (r.target_object_id, 0.0, 0, 0, 0),
            protocol::GameMessage::Hit(r) if r.defender & 0xff000000 == 0x0d000000 => {
                let profile = self.bullet_profile(active, r);
                let result = profile.and_then(|p| self.item_catalog.vehicle_bullet(p.item_id()));
                raw = Some((r.penetration_fraction(), result));
                (r.defender, 0.0, r.hit_location, r.client_data, 1)
            }
            protocol::GameMessage::ObjectUpdate(update)
                if update.header.nak_sequence == 0 && !self.pending_bunker_damage.is_empty() =>
            {
                let &(index, hp, _) = self.pending_bunker_damage.front().unwrap();
                pending = true;
                (0x0d800000 | u32::from(index), hp, 0, 0, 1)
            }
            _ => return Ok(None),
        };
        let result = (|| {
            let w = w.ok_or("bunker_world_unavailable")?;
            let index = w.index(target).ok_or("unknown_bunker")?;
            let bunker = &w.bunkers[&index];
            if !active.entered_game
                || !active.ready_for_updates
                || (!pending && !critical && !active.life.is_alive())
            {
                return Err("bunker_requires_living_ready_actor");
            }
            let mut next = bunker.clone();
            let mut amount = amount;
            if repair {
                if active.mounted_seat.is_some()
                    || bunker.faction != active.reservation.character.faction
                {
                    return Err("repair_requires_allied_onfoot_actor");
                }
                if now
                    < self
                        .next_repair
                        .get(&active.reservation.id)
                        .copied()
                        .unwrap_or_default()
                {
                    return Err("repair_cooldown");
                }
                let handle = active
                    .current_inventory_handle
                    .ok_or("repair_inventory_unavailable")?;
                let kit = active
                    .reservation
                    .character
                    .loadout(
                        &self.item_catalog,
                        handle.inventory_index,
                        active.wire_profile,
                    )
                    .ok_or("repair_inventory_unavailable")?;
                let observed = if active.primary_equipped {
                    active.observed_primary
                } else {
                    active.observed_secondary
                }
                .ok_or("repair_tool_not_equipped")?;
                if observed.inventory_update_id != handle.update_id
                    || kit.inventories[usize::from(handle.inventory_index)]
                        .slots
                        .get(usize::from(observed.gear_slot))
                        .copied()
                        .flatten()
                        != Some(w.catalog.repair().item_id)
                {
                    return Err("repair_tool_not_equipped");
                }
                if bunker.health.conditions() & 4 != 0 && !bunker.health.destroyed {
                    return Err("critical_bunker_repair_requires_destruction");
                }
                if bunker.health.hp() == bunker.profile.max_health {
                    return Err("bunker_already_healthy");
                }
                if bunker.health.destroyed && !bunker.profile.can_repair_destroyed {
                    return Err("bunker_cannot_rebuild");
                }
                // Native repair selects the vehicle/turret through the client's
                // authored body raycast. Entry points are not repair surfaces;
                // retain their distance only as diagnostics, never an admission gate.
                repair_mount_distance = active
                    .tracking
                    .sample
                    .map(|sample| {
                        let pos = [sample.position.x, sample.position.y, sample.position.z];
                        bunker
                            .repair_points
                            .iter()
                            .map(|p| {
                                p.iter()
                                    .zip(pos)
                                    .map(|(a, b)| (*a - b).powi(2))
                                    .sum::<f32>()
                                    .sqrt()
                            })
                            .fold(f32::INFINITY, f32::min)
                    })
                    .filter(|distance| distance.is_finite());
                amount = w.catalog.repair_amount(
                    &bunker.profile,
                    active.modifiers.value("kCharModRepairAmount"),
                )?;
                next.health.repair(amount)?;
            } else {
                if bunker.health.hp() == 0.0 {
                    return Err("bunker_destroyed");
                }
                if active.mounted_seat.is_some_and(|seat| seat != 0) && !pending {
                    return Err("bunker_hit_requires_armed_actor");
                }
                if bunker.health.conditions() & 4 != 0 && !pending && !critical {
                    return Err("critical_bunker_damage_over_time");
                }
                if let Some((scale, params)) = raw {
                    let params = params?;
                    amount = w.damage_amount(
                        index,
                        (params.amount * scale)
                            * if bunker.team == 1 {
                                w.catalog.friendly_fire_multiplier()
                            } else {
                                1.0
                            },
                        params.armor_pierce,
                        params.armor_redux,
                        location,
                    )?;
                }
                next.health.damage(amount)?;
            }
            next.last_health_update = now;
            if !repair && !critical {
                next.damage = VehicleDamage {
                    last_location_and_type: (kind << 4) | location,
                    health_quarter_points: next.health.wire_health(),
                    conditions: next.health.conditions(),
                    last_attacker: 0x03800000 | u32::from(active.reservation.player),
                    last_source_object: u32::MAX,
                    last_user_data: userdata,
                };
            }
            next.damage.health_quarter_points = next.health.wire_health();
            next.damage.conditions = next.health.conditions();
            let packet = BunkerDelta {
                index,
                damage: next.damage,
            }
            .encode_with_profile(
                active.next_sequence()?,
                now.as_millis() as u32,
                self.wire_profile,
            )?;
            Ok::<_, &'static str>((index, next, amount, packet))
        })();
        let id = active.reservation.id.clone();
        let world = active.reservation.world;
        let player = active.reservation.player;
        match result {
            Err(code) => {
                self.record(json!({"op":"structure_event","reservation_id":id,"type":if repair {"repair"}else if critical {"bunker_critical_decay"}else{"bunker_hit"},"target_object_id":target,"gameplay_denial":code}),wall)?;
                if pending && code != "bunker_requires_living_ready_actor" {
                    self.pending_bunker_damage.pop_front();
                }
                Ok(Some(Vec::new()))
            }
            Ok((index, next, amount, packet)) => {
                self.record(json!({"op":"structure_event","reservation_id":id,"type":if repair {"repair"}else if critical {"bunker_critical_decay"}else{"bunker_hit"},"target_object_id":target,"catalog_id":next.id,"amount_hp":amount,"health_hp":next.health.hp(),"health_quarter_points":next.damage.health_quarter_points,"conditions":next.damage.conditions,"repair_mount_distance":repair_mount_distance,"policy":"native_vehicle_armor_float_health_client_repair_target"}),wall)?;
                self.active.as_mut().unwrap().remember(packet.clone())?;
                self.active.as_mut().unwrap().last_event = now;
                self.bunker_world
                    .as_mut()
                    .unwrap()
                    .bunkers
                    .insert(index, next);
                if repair {
                    self.next_repair.insert(
                        id.clone(),
                        now + Duration::from_secs_f32(
                            self.bunker_world
                                .as_ref()
                                .unwrap()
                                .catalog
                                .repair()
                                .cooldown_seconds,
                        ),
                    );
                }
                if pending {
                    self.pending_bunker_damage.pop_front();
                }
                Ok(Some(vec![OutboundMessage {
                    reservation_id: id,
                    world_id: world,
                    player_id: player,
                    plugin_id: 1,
                    payload_hex: protocol::encode_hex(&packet),
                }]))
            }
        }
    }
}
