//! Source-qualified indexed Domination hospital turret authority.

use crate::{
    Active, CharacterLife, Engine, Write, bullet_hit,
    hospital_turret_wire::{self as wire, ClientUpdate, Fields, InventoryDefinition, ServerUpdate},
    item_catalog::ItemCatalog,
    replication::InventoryItem,
    spawn_catalog::SpawnCatalog,
    structures::BunkerWorld,
};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rules {
    schema: String,
    source: Source,
    variants: Vec<Variant>,
    profiles: Vec<Profile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    structure_sha256: String,
    item_catalog_sha256: String,
    retail_elf_sha256: String,
    retail_archive_sha256: String,
    vehicle_type_token: u32,
    seat_pose_sha256: String,
    wire_proof_sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Variant {
    faction: u8,
    primary_item_id: u16,
    secondary_item_id: u16,
    properties_source: String,
    seat_node: String,
    seat_index: u8,
    turret_index: u8,
    seat_pose_scope: String,
    occupant_visible: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    map_name: String,
    spawn_catalog_sha256: String,
    placements: Vec<Placement>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Placement {
    object_index: u16,
    spawn_index: u16,
    spawn_token: u32,
    faction: u8,
}

pub(crate) fn qualify(
    bytes: &[u8],
    bunkers: &BunkerWorld,
    spawns: &SpawnCatalog,
    items: &ItemCatalog,
    native_elf: &str,
) -> std::io::Result<BTreeMap<u16, Runtime>> {
    let rules: Rules = serde_json::from_slice(bytes).map_err(std::io::Error::other)?;
    rules
        .bind(bunkers, spawns, items, native_elf)
        .map_err(std::io::Error::other)
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

impl Source {
    fn matches(
        &self,
        structure_hash: Option<&str>,
        native_elf: &str,
        vehicle_type_token: u32,
        provenance: &serde_json::Value,
    ) -> bool {
        [
            &self.structure_sha256,
            &self.item_catalog_sha256,
            &self.retail_elf_sha256,
            &self.retail_archive_sha256,
            &self.seat_pose_sha256,
            &self.wire_proof_sha256,
        ]
        .into_iter()
        .all(|digest| valid_hash(digest))
            && structure_hash == Some(self.structure_sha256.as_str())
            && native_elf == self.retail_elf_sha256
            && self.vehicle_type_token != 0
            && vehicle_type_token == self.vehicle_type_token
            && provenance["source_archive_sha256"].as_str()
                == Some(self.retail_archive_sha256.as_str())
            && provenance["catalog_sha256"].as_str() == Some(self.item_catalog_sha256.as_str())
    }
}

impl Rules {
    fn bind(
        &self,
        bunkers: &BunkerWorld,
        spawns: &SpawnCatalog,
        items: &ItemCatalog,
        native_elf: &str,
    ) -> Result<BTreeMap<u16, Runtime>, &'static str> {
        let combat = items.retail_combat()?;
        if self.schema != "mag_domination_hospital_turrets_v1"
            || !self.source.matches(
                bunkers.domination_structure_sha256(),
                native_elf,
                bunkers.vehicle_type_token(),
                combat.provenance(),
            )
        {
            return Err("hospital_turret_source_mismatch");
        }
        let mut variants = BTreeMap::new();
        for row in &self.variants {
            let namespace = format!("FactionItemList{}:", u16::from(row.faction) + 1);
            let primary = combat
                .hospital_primary_for_damage(row.primary_item_id)
                .map_err(|_| "hospital_turret_primary_missing")?;
            if row.faction >= 3
                || row.properties_source.is_empty()
                || row.seat_node != "gunner_seat"
                || row.seat_index != 0
                || row.turret_index != 0
                || row.seat_pose_scope != "retail_root0_identity_fallback"
                || row.occupant_visible
                || row.primary_item_id == row.secondary_item_id
                || [row.primary_item_id, row.secondary_item_id]
                    .into_iter()
                    .any(|id| id == 0 || id >= 4096)
                || !primary.authored_ref.starts_with(&namespace)
                || items.retail_authored_ref(row.primary_item_id)
                    != Some(primary.authored_ref.as_str())
                || !items
                    .retail_authored_ref(row.secondary_item_id)
                    .is_some_and(|id| id.starts_with(&namespace))
                || variants.insert(row.faction, row).is_some()
            {
                return Err("invalid_hospital_turret_variant");
            }
        }
        if variants.len() != 3 {
            return Err("invalid_hospital_turret_variant");
        }
        let mut profiles = BTreeSet::new();
        let mut active = None;
        for profile in &self.profiles {
            let mut indices = BTreeSet::new();
            let mut spawn_indices = BTreeSet::new();
            if profile.map_name.is_empty()
                || !valid_hash(&profile.spawn_catalog_sha256)
                || !profiles.insert(&profile.spawn_catalog_sha256)
                || profile.placements.is_empty()
                || profile.placements.iter().any(|row| {
                    row.object_index >= 16
                        || row.spawn_index >= 128
                        || row.spawn_token == 0
                        || row.faction >= 3
                        || !indices.insert(row.object_index)
                        || !spawn_indices.insert(row.spawn_index)
                })
            {
                return Err("invalid_hospital_turret_profile");
            }
            if spawns.source_sha256() == Some(profile.spawn_catalog_sha256.as_str())
                && spawns.map_name() == profile.map_name
            {
                active = Some(profile);
            }
        }
        if profiles.len() != 6 {
            return Err("invalid_hospital_turret_profile");
        }
        let active = active.ok_or("hospital_turret_profile_required")?;
        if active.placements.len() != bunkers.active_count() {
            return Err("hospital_turret_placement_mismatch");
        }
        active
            .placements
            .iter()
            .map(|row| {
                let bunker = bunkers
                    .domination_bunker(row.object_index)
                    .ok_or("hospital_turret_placement_mismatch")?;
                let variant = variants[&row.faction];
                if (bunker.faction, bunker.spawn_index) != (row.faction, row.spawn_index)
                    || bunker.properties_source != variant.properties_source
                    || !spawns
                        .point(row.spawn_index)
                        .is_some_and(|point| point.token == row.spawn_token)
                {
                    return Err("hospital_turret_placement_mismatch");
                }
                Ok((
                    row.object_index,
                    Runtime::new(variant.primary_item_id, variant.secondary_item_id),
                ))
            })
            .collect()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Runtime {
    items: [Option<u16>; 3],
    version: u8,
    aim: Fields,
    grant: Option<Grant>,
}

#[derive(Clone, Debug)]
struct Grant {
    reservation: String,
    player: u16,
    generation: u32,
    primary: Option<InventoryItem>,
    secondary: Option<InventoryItem>,
}

impl Runtime {
    fn new(primary: u16, secondary: u16) -> Self {
        Self {
            items: [Some(primary), Some(secondary), None],
            version: 2,
            aim: Fields::default(),
            grant: None,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.version = 2;
        self.aim = Fields::default();
        self.grant = None;
    }

    pub(crate) fn revoke(&mut self) {
        self.grant = None;
    }

    pub(crate) fn yaw(&self) -> Option<f32> {
        self.aim.yaw.map(|yaw| yaw.radians())
    }

    pub(crate) fn mount(
        &self,
        actor: &Active,
    ) -> Result<(Self, InventoryDefinition), &'static str> {
        let CharacterLife::Alive { generation, .. } = actor.life else {
            return Err("hospital_turret_requires_living_owner");
        };
        let mut next = self.clone();
        next.version = self
            .version
            .checked_add(1)
            .ok_or("hospital_turret_grants_exhausted")?;
        next.grant = Some(Grant {
            reservation: actor.reservation.id.clone(),
            player: actor.reservation.player,
            generation,
            primary: None,
            secondary: None,
        });
        let definition = next.definition(true);
        Ok((next, definition))
    }

    fn definition(&self, granted: bool) -> InventoryDefinition {
        InventoryDefinition {
            version: if granted { self.version } else { 2 },
            items: self.items,
        }
    }

    fn grant_for(&self, actor: &Active) -> Option<&Grant> {
        let grant = self.grant.as_ref()?;
        let CharacterLife::Alive { generation, .. } = actor.life else {
            return None;
        };
        (grant.generation == generation
            && grant.player == actor.reservation.player
            && grant.reservation == actor.reservation.id)
            .then_some(grant)
    }

    pub(crate) fn accepted_actor(&self, actor: &Active, index: u16) -> bool {
        self.grant.is_none()
            || (actor.mounted_bunker == Some(index)
                && actor.mounted_seat == Some(0)
                && self.grant_for(actor).is_some())
    }

    fn can_fire(&self, equipped: InventoryItem) -> bool {
        equipped.inventory_update_id == self.version
            && equipped.gear_slot == 0
            && equipped.ammo_pool == 0
    }

    fn report(&self, fields: Fields) -> Result<(Self, Fields), &'static str> {
        if [fields.primary, fields.secondary]
            .into_iter()
            .flatten()
            .any(|item| {
                item.inventory_update_id != self.version
                    || self
                        .items
                        .get(usize::from(item.gear_slot))
                        .is_none_or(Option::is_none)
            })
        {
            return Err("hospital_turret_stale_equipment");
        }
        let mut next = self.clone();
        let grant = next
            .grant
            .as_mut()
            .ok_or("hospital_turret_grant_required")?;
        if let Some(primary) = fields.primary {
            grant.primary = Some(primary);
        }
        if let Some(secondary) = fields.secondary {
            grant.secondary = Some(secondary);
        }
        let ready = grant.primary.is_some_and(|primary| self.can_fire(primary));
        if !ready && fields.primary.is_none() {
            return Err("hospital_turret_primary_not_initialized");
        }
        let mut relay = fields;
        if ready {
            next.aim.yaw = fields.yaw.or(next.aim.yaw);
            next.aim.pitch = fields.pitch.or(next.aim.pitch);
            next.aim.overheat_info = fields.overheat_info.or(next.aim.overheat_info);
        } else {
            relay.yaw = None;
            relay.pitch = None;
            relay.overheat_info = None;
        }
        Ok((next, relay))
    }
}

pub(crate) struct QualifiedPrimary {
    pub(crate) item_id: u16,
    pub(crate) equipped: InventoryItem,
    pub(crate) definition_version: u8,
}

/// Definitions and aim fit in one packet, followed by at most 16 owner rows.
pub(crate) const MAX_LATE_JOIN_PACKETS: usize = 17;

impl<W: Write> Engine<W> {
    pub(crate) fn has_hospital_turrets(&self) -> bool {
        self.bunker_world
            .as_ref()
            .is_some_and(|world| !world.hospital_turrets.is_empty())
    }

    fn hospital_authority(
        &self,
        actor: &Active,
        index: u16,
    ) -> Result<(&Runtime, &Grant), &'static str> {
        let world = self
            .bunker_world
            .as_ref()
            .ok_or("hospital_turret_world_required")?;
        let bunker = world
            .domination_bunker(index)
            .ok_or("hospital_turret_unknown_index")?;
        let turret = world
            .hospital_turrets
            .get(&index)
            .ok_or("hospital_turret_unqualified")?;
        if !bunker.operational
            || !actor.entered_game
            || !actor.ready_for_updates
            || actor.emplacement_creation.is_none()
            || actor.mounted_bunker != Some(index)
            || actor.mounted_seat != Some(0)
            || actor.team() != bunker.team
            || actor.reservation.character.faction != bunker.faction
            || self
                .active
                .iter()
                .chain(self.peers.values())
                .filter(|peer| peer.mounted_bunker == Some(index) && peer.mounted_seat == Some(0))
                .count()
                != 1
        {
            return Err("hospital_turret_requires_living_exclusive_owner");
        }
        let grant = turret
            .grant_for(actor)
            .ok_or("hospital_turret_grant_required")?;
        Ok((turret, grant))
    }

    pub(crate) fn revoke_inactive_hospital_turrets(&mut self) {
        if let Some(world) = &self.bunker_world {
            let stale = world
                .hospital_turrets
                .iter()
                .filter_map(|(&index, turret)| {
                    (turret.grant.is_some()
                        && !self
                            .active
                            .iter()
                            .chain(self.peers.values())
                            .any(|actor| self.hospital_authority(actor, index).is_ok()))
                    .then_some(index)
                })
                .collect::<Vec<_>>();
            let world = self.bunker_world.as_mut().unwrap();
            for index in stale {
                world.hospital_turrets.get_mut(&index).unwrap().revoke();
            }
        }
    }

    pub(crate) fn hospital_primary_for_actor(
        &self,
        actor: &Active,
        hit: &bullet_hit::NetMessageHit,
    ) -> Result<QualifiedPrimary, &'static str> {
        let index = actor
            .mounted_bunker
            .ok_or("hospital_turret_unknown_index")?;
        let (turret, grant) = self.hospital_authority(actor, index)?;
        if hit.gear_index != 0 || hit.fire_mode != 0 {
            return Err("unsupported_hospital_turret_fire_mode");
        }
        let equipped = grant
            .primary
            .filter(|item| turret.can_fire(*item))
            .ok_or("hospital_turret_primary_not_initialized")?;
        Ok(QualifiedPrimary {
            item_id: turret.items[0].ok_or("hospital_turret_primary_missing")?,
            equipped,
            definition_version: turret.version,
        })
    }

    pub(crate) fn prepare_hospital_turret_report(
        &self,
        actor: &Active,
        update: ClientUpdate,
        now: Duration,
    ) -> Result<(Runtime, Vec<Vec<u8>>), &'static str> {
        let (turret, _) = self.hospital_authority(actor, update.index)?;
        let (next, fields) = turret.report(update.fields)?;
        let packets = wire::encode_updates(
            1,
            now.as_millis() as u32,
            &[ServerUpdate {
                index: update.index,
                fields,
                definition: None,
            }],
            self.wire_profile,
        )?;
        Ok((next, packets))
    }

    pub(crate) fn hospital_turret_late_join(
        &self,
        now: Duration,
    ) -> Result<Vec<Vec<u8>>, &'static str> {
        let mut rows = Vec::new();
        let mut ownership = Vec::new();
        if let Some(world) = &self.bunker_world {
            for (&index, turret) in &world.hospital_turrets {
                let gunner = self
                    .active
                    .iter()
                    .chain(self.peers.values())
                    .find_map(|actor| {
                        self.hospital_authority(actor, index)
                            .ok()
                            .map(|(_, grant)| grant)
                    });
                let operational = world
                    .domination_bunker(index)
                    .is_some_and(|bunker| bunker.operational);
                let mut fields = if operational {
                    turret.aim
                } else {
                    Fields::default()
                };
                if let Some(grant) = gunner {
                    fields.primary = grant.primary;
                    fields.secondary = grant.secondary;
                    ownership.push(
                        self.bunker_ownership_after(index, u16::MAX, None)
                            .encode_bunker_update_with_profile(
                                index,
                                1,
                                now.as_millis() as u32,
                                self.wire_profile,
                            )?,
                    );
                }
                rows.push(ServerUpdate {
                    index,
                    fields,
                    definition: Some(turret.definition(gunner.is_some())),
                });
            }
        }
        let mut packets =
            wire::encode_updates(1, now.as_millis() as u32, &rows, self.wire_profile)?;
        packets.extend(ownership);
        if packets.len() > MAX_LATE_JOIN_PACKETS {
            return Err("hospital_turret_late_join_budget_exceeded");
        }
        Ok(packets)
    }

    pub(crate) fn check_hospital_turret_queue<'a>(
        &self,
        recipients: impl Iterator<Item = &'a Active>,
        packets: usize,
        baseline_slots: impl Fn(&Active) -> usize,
    ) -> Result<(), &'static str> {
        if packets == 0 {
            return Ok(());
        }
        let forward_spawns = self.pending_forward_spawns();
        if recipients.into_iter().any(|actor| {
            let forward_slots = if forward_spawns.is_empty() {
                0
            } else {
                // Availability, one assignment per rule and at most one
                // personal assignment per rule for the recipient's squad.
                1 + forward_spawns.len()
                    + forward_spawns
                        .iter()
                        .filter(|rule| rule.squad_index == actor.squad_catalog.squad_index())
                        .count()
            };
            actor.replication_backlogged
                || actor.ordinary_shared_len() + baseline_slots(actor) + forward_slots + packets
                    > crate::multiplayer::MAX_SHARED_QUEUE
        }) {
            return Err("hospital_turret_publication_budget_exceeded");
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "hospital_turret_runtime_tests.rs"]
pub(crate) mod tests;
