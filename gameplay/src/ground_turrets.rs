//! Qualified turret-zero authority for physical seat one of movable Ground vehicles.
use crate::{
    Active, Engine, Write, bullet_hit,
    ground_vehicles::{Catalog, OwnerUpdate, TurretUpdate},
    hospital_turret_wire::InventoryDefinition,
    hospital_turrets::Runtime as Turret,
    item_catalog::ItemCatalog,
    replication::{CharacterPosition, InventoryItem},
    spawn_catalog::SpawnCatalog,
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
    item_catalog_sha256: String,
    retail_elf_sha256: String,
    retail_archive_sha256: String,
    retail_main_sha256: String,
    pose_proof_sha256: String,
    wire_proof_sha256: String,
    vehicle_type_token: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Variant {
    faction: u8,
    primary_item_id: u16,
    seat_node: String,
    seat_index: u8,
    turret_index: u8,
    occupant_visible: bool,
    local_position_xyz: [f32; 3],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    map_name: String,
    spawn_catalog_sha256: String,
    ground_catalog_sha256: String,
    vehicles: Vec<Placement>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Placement {
    vehicle_index: u16,
    spawn_point_index: u16,
    spawn_point_token: u32,
    faction: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PolicyKind {
    Apc,
    Escort,
}

impl PolicyKind {
    fn for_schema(schema: &str, game_type: i8) -> Result<Self, &'static str> {
        match (schema, game_type) {
            ("mag_domination_apc_turrets_v1", 2) | ("mag_acquisition_apc_turrets_v1", 1) => {
                Ok(Self::Apc)
            }
            ("mag_acquisition_escort_turrets_v1", 1) => Ok(Self::Escort),
            _ => Err("ground_turret_source_mismatch"),
        }
    }

    fn spawn_type(self) -> u8 {
        match self {
            Self::Apc => 3,
            Self::Escort => 9,
        }
    }

    fn placements(self, game_type: i8) -> usize {
        match self {
            Self::Apc if game_type == 2 => 8,
            Self::Apc => 4,
            Self::Escort => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Policy {
    kind: PolicyKind,
    primary: u16,
    local_position_xyz: [f32; 3],
}

impl Policy {
    pub(crate) fn initial_turret(self) -> Turret {
        Turret::new(self.primary, None)
    }

    pub(crate) fn position(
        self,
        vehicle: &crate::ground_vehicles::Creation,
    ) -> Result<CharacterPosition, &'static str> {
        let transform = crate::vehicle_exit::BunkerTransform {
            position: vehicle.position,
            rotation_xyzw: vehicle.rotation,
        };
        let [x, y, z] = transform.rotate(self.local_position_xyz);
        let position = CharacterPosition {
            x: vehicle.position.x + x,
            y: vehicle.position.y + y,
            z: vehicle.position.z + z,
        };
        position.validate()?;
        Ok(position)
    }
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(crate) fn qualify(
    bytes: &[u8],
    grounds: &Catalog,
    spawns: &SpawnCatalog,
    items: &ItemCatalog,
    native_elf: &str,
) -> std::io::Result<BTreeMap<u16, Policy>> {
    let rules: Rules = serde_json::from_slice(bytes).map_err(std::io::Error::other)?;
    rules
        .bind(grounds, spawns, items, native_elf)
        .map_err(std::io::Error::other)
}

/// Prepare both Acquisition bindings before installing either one.
pub(crate) fn bind_acquisition(
    grounds: &mut Catalog,
    spawns: &SpawnCatalog,
    items: &ItemCatalog,
    native_elf: &str,
    escort_source: Option<&[u8]>,
    apc_source: Option<&[u8]>,
) -> std::io::Result<()> {
    use std::io::Error;
    if grounds.has_acquisition_apc_crew() != apc_source.is_some() {
        return Err(Error::other("acquisition_apc_crew_source_mismatch"));
    }
    if grounds.has_acquisition_apc_crew() && grounds.native_elf_sha256() != native_elf {
        return Err(Error::other("acquisition_apc_crew_source_mismatch"));
    }
    if grounds.has_acquisition_apc_crew() && escort_source.is_none() {
        return Err(Error::other("acquisition_apc_crew_requires_escort_turrets"));
    }
    let mut combined = BTreeMap::new();
    for (source, expected_schema) in [
        (escort_source, "mag_acquisition_escort_turrets_v1"),
        (apc_source, "mag_acquisition_apc_turrets_v1"),
    ] {
        let Some(raw) = source else {
            continue;
        };
        let rules: Rules = serde_json::from_slice(raw).map_err(Error::other)?;
        if rules.schema != expected_schema {
            return Err(Error::other("ground_turret_source_mismatch"));
        }
        for (index, policy) in rules
            .bind(grounds, spawns, items, native_elf)
            .map_err(Error::other)?
        {
            if combined.insert(index, policy).is_some() {
                return Err(Error::other("duplicate_acquisition_ground_turret_policy"));
            }
        }
    }
    grounds.set_turret_policies(combined);
    Ok(())
}

impl Rules {
    fn bind(
        &self,
        grounds: &Catalog,
        spawns: &SpawnCatalog,
        items: &ItemCatalog,
        native_elf: &str,
    ) -> Result<BTreeMap<u16, Policy>, &'static str> {
        let combat = items.retail_combat()?;
        let source = &self.source;
        let game_type = spawns.match_profile().game_type;
        let kind = PolicyKind::for_schema(&self.schema, game_type)?;
        if game_type == 1 && kind == PolicyKind::Apc && !grounds.has_acquisition_apc_crew() {
            return Err("acquisition_apc_crew_catalog_required");
        }
        let expected_placements = kind.placements(game_type);
        if [
            &source.item_catalog_sha256,
            &source.retail_elf_sha256,
            &source.retail_archive_sha256,
            &source.retail_main_sha256,
            &source.pose_proof_sha256,
            &source.wire_proof_sha256,
        ]
        .into_iter()
        .any(|digest| !valid_hash(digest))
            || native_elf != source.retail_elf_sha256
            || source.vehicle_type_token == 0
            || combat.provenance()["catalog_sha256"].as_str()
                != Some(source.item_catalog_sha256.as_str())
            || combat.provenance()["source_archive_sha256"].as_str()
                != Some(source.retail_archive_sha256.as_str())
        {
            return Err("ground_turret_source_mismatch");
        }
        let mut variants = BTreeMap::new();
        for variant in &self.variants {
            let primary = match kind {
                PolicyKind::Apc => combat.ground_apc_primary_for_damage(variant.primary_item_id),
                PolicyKind::Escort => {
                    combat.ground_escort_primary_for_damage(variant.primary_item_id)
                }
            }
            .map_err(|_| "ground_turret_primary_missing")?;
            let namespace = format!("FactionItemList{}:", u16::from(variant.faction) + 1);
            if variant.faction >= 3
                || variant.seat_index != 1
                || variant.turret_index != 0
                || variant.seat_node != "turret_01"
                || variant.occupant_visible
                || variant.local_position_xyz == [0.; 3]
                || !variant
                    .local_position_xyz
                    .iter()
                    .all(|position| position.is_finite())
                || !primary.authored_ref.starts_with(&namespace)
                || items.retail_authored_ref(variant.primary_item_id)
                    != Some(primary.authored_ref.as_str())
                || variant.primary_item_id == 0
                || variant.primary_item_id >= 4096
                || variants.insert(variant.faction, variant).is_some()
            {
                return Err("invalid_ground_turret_variant");
            }
        }
        if variants.len() != 3 {
            return Err("invalid_ground_turret_variant");
        }

        let mut spawn_hashes = BTreeSet::new();
        let mut ground_hashes = BTreeSet::new();
        let mut active = None;
        for profile in &self.profiles {
            let mut vehicles = BTreeSet::new();
            let mut points = BTreeSet::new();
            if profile.map_name.is_empty()
                || !valid_hash(&profile.spawn_catalog_sha256)
                || !valid_hash(&profile.ground_catalog_sha256)
                || !spawn_hashes.insert(&profile.spawn_catalog_sha256)
                || !ground_hashes.insert(&profile.ground_catalog_sha256)
                || profile.vehicles.len() != expected_placements
                || profile.vehicles.iter().any(|vehicle| {
                    vehicle.vehicle_index >= 32
                        || vehicle.spawn_point_index >= 128
                        || vehicle.spawn_point_token == 0
                        || vehicle.faction >= 3
                        || !vehicles.insert(vehicle.vehicle_index)
                        || !points.insert(vehicle.spawn_point_index)
                })
            {
                return Err("invalid_ground_turret_profile");
            }
            if spawns.source_sha256() == Some(profile.spawn_catalog_sha256.as_str())
                && grounds.raw_sha256() == profile.ground_catalog_sha256
                && spawns.map_name() == profile.map_name
            {
                active = Some(profile);
            }
        }
        if self.profiles.len() != 6 {
            return Err("invalid_ground_turret_profile");
        }
        let active = active.ok_or("ground_turret_profile_required")?;
        if game_type == 1
            && grounds
                .creations()
                .filter(|vehicle| grounds.source_spawn_type(vehicle) == Some(kind.spawn_type()))
                .count()
                != expected_placements
        {
            return Err("ground_turret_placement_mismatch");
        }
        active
            .vehicles
            .iter()
            .map(|row| {
                let vehicle = grounds
                    .vehicle(row.vehicle_index)
                    .ok_or("ground_turret_placement_mismatch")?;
                if grounds.source_spawn_type(vehicle) != Some(kind.spawn_type())
                    || (kind == PolicyKind::Escort
                        && (vehicle.maximum_occupancy != Some(8)
                            || !vehicle.passenger_seats.iter().copied().eq(2..=9)))
                    || grounds
                        .driver_seat(row.vehicle_index)
                        .is_none_or(|driver| driver.seat != 0)
                    || grounds
                        .seat_occupant_visible(row.vehicle_index, 0)
                        .is_none()
                    || (
                        vehicle.spawn_point_index,
                        vehicle.spawn_point_token,
                        vehicle.faction,
                        vehicle.vehicle_type_token,
                    ) != (
                        row.spawn_point_index,
                        row.spawn_point_token,
                        row.faction,
                        source.vehicle_type_token,
                    )
                {
                    return Err("ground_turret_placement_mismatch");
                }
                let variant = variants[&row.faction];
                let policy = Policy {
                    kind,
                    primary: variant.primary_item_id,
                    local_position_xyz: variant.local_position_xyz,
                };
                policy.position(vehicle)?;
                Ok((row.vehicle_index, policy))
            })
            .collect()
    }
}

pub(crate) struct GroundQualifiedPrimary {
    pub(crate) kind: PolicyKind,
    pub(crate) item_id: u16,
    pub(crate) equipped: InventoryItem,
    pub(crate) definition_version: u8,
}

impl<W: Write> Engine<W> {
    pub(crate) fn has_ground_turrets(&self) -> bool {
        self.ground_vehicles
            .as_ref()
            .is_some_and(Catalog::has_turrets)
    }

    pub(crate) fn ground_turret_policy(&self, index: u16) -> Option<Policy> {
        self.ground_vehicles.as_ref()?.turret_policy(index)
    }

    pub(crate) fn is_ground_gunner(&self, actor: &Active) -> bool {
        self.ground_gunner_kind(actor).is_some()
    }

    pub(crate) fn ground_gunner_kind(&self, actor: &Active) -> Option<PolicyKind> {
        let seat = actor.ground_seat.filter(|seat| seat.seat == 1)?;
        Some(self.ground_turret_policy(seat.vehicle_index)?.kind)
    }

    fn ground_turret(&self, index: u16) -> Option<&Turret> {
        self.ground_runtime.as_ref()?.get(index)?.turret.as_ref()
    }

    fn ground_gunner_authority(&self, actor: &Active, index: u16) -> bool {
        self.is_ground_gunner(actor)
            && actor
                .ground_seat
                .is_some_and(|seat| seat.vehicle_index == index)
            && actor.entered_game
            && actor.ready_for_updates
            && actor.life.is_alive()
            && self.current_ground_vehicle(index).is_some_and(|vehicle| {
                vehicle.team == actor.team()
                    && vehicle.faction == actor.reservation.character.faction
            })
            && self.ground_vehicle_is_operational(index)
            && self.ground_vehicle_creation_complete(actor.ground_vehicle_batch)
            && self
                .ground_turret(index)
                .is_some_and(|turret| turret.grant_matches(actor))
            && self
                .active
                .iter()
                .chain(self.peers.values())
                .filter(|peer| peer.ground_seat == actor.ground_seat)
                .count()
                == 1
    }

    pub(crate) fn ground_primary_for_actor(
        &self,
        actor: &Active,
        hit: &bullet_hit::NetMessageHit,
    ) -> Result<GroundQualifiedPrimary, &'static str> {
        let index = actor
            .ground_seat
            .map(|seat| seat.vehicle_index)
            .ok_or("ground_turret_seat_required")?;
        if !self.ground_gunner_authority(actor, index) {
            return Err("ground_turret_requires_living_exclusive_gunner");
        }
        if hit.gear_index != 0 || hit.fire_mode != 0 {
            return Err("unsupported_ground_turret_fire_mode");
        }
        let (item_id, equipped, definition_version) = self
            .ground_turret(index)
            .and_then(|turret| turret.primary_for(actor))
            .ok_or("ground_turret_primary_not_initialized")?;
        let kind = self
            .ground_gunner_kind(actor)
            .ok_or("ground_turret_seat_required")?;
        Ok(GroundQualifiedPrimary {
            kind,
            item_id,
            equipped,
            definition_version,
        })
    }

    pub(crate) fn ground_turret_seat_change(
        &self,
        actor: &Active,
        index: u16,
        entering: bool,
    ) -> Result<(Turret, Option<InventoryDefinition>), &'static str> {
        let mut turret = self
            .ground_turret(index)
            .ok_or("ground_turret_seat_required")?
            .clone();
        if entering {
            let (next, definition) = turret.mount(actor)?;
            return Ok((next, Some(definition)));
        }
        turret.revoke();
        Ok((turret, None))
    }

    pub(crate) fn ground_turret_snapshot(&self, index: u16) -> Option<TurretUpdate> {
        let turret = self.ground_turret(index)?;
        let gunner = self
            .active
            .iter()
            .chain(self.peers.values())
            .find(|actor| self.ground_gunner_authority(actor, index));
        Some(TurretUpdate {
            index,
            driver_owner: None,
            turret_owner: Some(gunner.map_or(u16::MAX, |actor| actor.reservation.player)),
            definition: Some(turret.definition(gunner.is_some())),
            fields: turret.snapshot(gunner, self.ground_vehicle_is_operational(index)),
        })
    }

    pub(crate) fn prepare_ground_turret_report(
        &self,
        actor: &Active,
        row: OwnerUpdate,
        now: Duration,
    ) -> Result<(Turret, Vec<Vec<u8>>), &'static str> {
        let index = row.dynamics.index;
        if !self.ground_gunner_authority(actor, index) {
            return Err("ground_turret_requires_living_exclusive_gunner");
        }
        let turret = self.ground_turret(index).unwrap();
        if row
            .turret
            .secondary
            .is_some_and(|item| !turret.is_empty_secondary(item))
        {
            return Err("unsupported_ground_turret_secondary");
        }
        let (next, fields) = turret.report(row.turret)?;
        let packets = TurretUpdate::encode_rows(
            1,
            now.as_millis() as u32,
            &[TurretUpdate {
                index,
                fields,
                driver_owner: None,
                turret_owner: None,
                definition: None,
            }],
            self.wire_profile,
        )?;
        Ok((next, packets))
    }

    pub(crate) fn active_ground_gunners(&self) -> Vec<(u16, u16)> {
        if !self.has_ground_turrets() {
            return Vec::new();
        }
        self.active
            .iter()
            .chain(self.peers.values())
            .filter_map(|actor| {
                let index = actor.ground_seat?.vehicle_index;
                self.ground_gunner_authority(actor, index)
                    .then_some((index, actor.reservation.player))
            })
            .collect()
    }

    pub(crate) fn revoke_ground_gunner(&mut self, index: u16) {
        if let Some(runtime) = &mut self.ground_runtime {
            runtime.revoke_turret(index);
        }
    }

    pub(crate) fn ground_turret_revocation_packet(
        &self,
        index: u16,
        now: Duration,
    ) -> Result<Vec<Vec<u8>>, &'static str> {
        TurretUpdate::encode_rows(
            1,
            now.as_millis() as u32,
            &[TurretUpdate {
                index,
                turret_owner: Some(u16::MAX),
                driver_owner: None,
                definition: None,
                fields: Default::default(),
            }],
            self.wire_profile,
        )
    }
}

#[cfg(test)]
#[path = "ground_turret_runtime_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
mod crew_scope_tests {
    use super::*;

    #[test]
    fn acquisition_apc_binding_is_distinct_from_escort_and_domination() {
        let schema = "mag_acquisition_apc_turrets_v1";
        assert_eq!(
            PolicyKind::for_schema(schema, 2),
            Err("ground_turret_source_mismatch")
        );
        assert_eq!(
            PolicyKind::for_schema("mag_domination_apc_turrets_v1", 1),
            Err("ground_turret_source_mismatch")
        );
        assert_eq!(PolicyKind::for_schema(schema, 1), Ok(PolicyKind::Apc));
    }
}
