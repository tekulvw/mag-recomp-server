//! Retail Helo damage and the shared on-foot vehicle hit input.
use crate::{item_catalog::TargetDamageParameters, *};
use std::collections::BTreeMap;

impl<W: Write> Engine<W> {
    pub(crate) fn retail_vehicle_bullet_parameters(
        &self,
        actor: &Active,
        request: &bullet_hit::NetMessageHit,
    ) -> Result<(u16, TargetDamageParameters), &'static str> {
        if !actor.entered_game
            || !actor.ready_for_updates
            || !actor.life.is_alive()
            || actor.is_mounted()
        {
            return Err("vehicle_hit_requires_living_onfoot_actor");
        }
        let CharacterHitProfile::Retail(profile) = self.bullet_profile(actor, request)? else {
            return Err("unsupported_vehicle_bullet_weapon");
        };
        let item_id = profile.item_id();
        let ammo = self
            .item_catalog
            .retail_combat()?
            .bullet_for_damage(item_id)
            .map_err(|_| "unsupported_vehicle_bullet_weapon")?;
        let damage = &ammo.damage;
        Ok((
            item_id,
            TargetDamageParameters {
                amount: damage.vehicle as f32,
                armor_pierce: damage.vehicle_armor_pierce as f32,
                armor_redux: damage.vehicle_armor_reduction as f32,
            },
        ))
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeloDamageRules {
    schema: String,
    retail_elf_sha256: String,
    models_sha256: String,
    grenade_material_penetration: f32,
    profiles: BTreeMap<u8, HeloDamageProfile>,
}

#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeloDamageProfile {
    pub(crate) max_health: f32,
    pub(crate) armor_type: u8,
    pub(crate) critical_loss_percent_per_second: f32,
    pub(crate) inbound_min_fraction: f32,
    pub(crate) outbound_min_fraction: f32,
    pub(crate) retreat_fraction: f32,
    pub(crate) destruction: Destruction,
}

#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Destruction {
    pub(crate) effect_seconds: f32,
    pub(crate) damage: f32,
    pub(crate) inner_radius: f32,
    pub(crate) outer_radius: f32,
    pub(crate) material_penetration: f32,
}

impl HeloDamageRules {
    pub(crate) fn from_json(
        raw: &[u8],
        retail_elf_sha256: &str,
        fleet: &domination_helo::FleetSource,
    ) -> Result<Self, &'static str> {
        if raw.len() > 65_536 {
            return Err("domination_helo_damage_source_too_large");
        }
        let rules: Self =
            serde_json::from_slice(raw).map_err(|_| "invalid_domination_helo_damage_source")?;
        if rules.schema != "mag_domination_helo_damage_v1"
            || rules.retail_elf_sha256 != retail_elf_sha256
            || rules.models_sha256 != fleet.source_receipt_sha256[2]
            || !rules.grenade_material_penetration.is_finite()
            || rules.grenade_material_penetration < 0.
            || !rules.profiles.keys().copied().eq(0..3)
            || rules.profiles.values().any(|p| !p.valid())
        {
            return Err("unbound_domination_helo_damage_source");
        }
        for carrier in &fleet.carriers {
            if rules.profile(carrier.faction)?.max_health * 4.0
                != f32::from(carrier.health_quarter_points)
            {
                return Err("domination_helo_damage_health_mismatch");
            }
        }
        Ok(rules)
    }

    pub(crate) fn profile(&self, faction: u8) -> Result<HeloDamageProfile, &'static str> {
        self.profiles
            .get(&faction)
            .copied()
            .ok_or("unbound_domination_helo_damage_faction")
    }
}

impl HeloDamageProfile {
    fn valid(self) -> bool {
        (1.0..=1023.75).contains(&self.max_health)
            && self.armor_type != 0
            && (0.0..=100.0).contains(&self.critical_loss_percent_per_second)
            && (0.0..=1.0).contains(&self.inbound_min_fraction)
            && (0.0..=1.0).contains(&self.outbound_min_fraction)
            && (0.0..=1.0).contains(&self.retreat_fraction)
            && [
                self.destruction.effect_seconds,
                self.destruction.damage,
                self.destruction.inner_radius,
                self.destruction.outer_radius,
                self.destruction.material_penetration,
            ]
            .into_iter()
            .all(|x| x.is_finite() && x >= 0.0)
            && self.destruction.inner_radius <= self.destruction.outer_radius
    }

    fn floor(self, flight: domination_helo::FlightState) -> f32 {
        use domination_helo::FlightState::*;
        match flight {
            Dormant => self.max_health,
            Inbound => self.max_health * self.inbound_min_fraction,
            Hover => 0.0,
            Outbound => self.max_health * self.outbound_min_fraction,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct HullDamage {
    hp: f32,
    pub(crate) wire: replication::VehicleDamage,
    pub(crate) state: replication::VehicleState,
    destroyed_age: f32,
}

#[derive(Clone, Copy)]
pub(crate) struct DamageDose {
    pub(crate) hp: f32,
    pub(crate) attacker: u32,
    pub(crate) location_and_type: u8,
    pub(crate) source_object: u32,
    pub(crate) user_data: u32,
}

impl HullDamage {
    pub(crate) fn fresh(quarters: u16) -> Self {
        Self {
            hp: f32::from(quarters) / 4.0,
            wire: replication::VehicleDamage {
                last_location_and_type: 0,
                health_quarter_points: quarters,
                conditions: 0,
                last_attacker: u32::MAX,
                last_source_object: u32::MAX,
                last_user_data: 0,
            },
            state: replication::VehicleState::Operational,
            destroyed_age: 0.,
        }
    }

    fn conditions(hp: f32, max: f32, thresholds: structure_catalog::NativeStructureRules) -> u8 {
        let percent = hp * 100.0 / max;
        u8::from(percent < thresholds.light_percent)
            | (u8::from(percent < thresholds.heavy_percent) << 1)
            | (u8::from(percent < thresholds.critical_percent) << 2)
            | (u8::from(hp == 0.0) << 3)
    }

    pub(crate) fn apply(
        &mut self,
        flight: domination_helo::FlightState,
        profile: HeloDamageProfile,
        thresholds: structure_catalog::NativeStructureRules,
        dose: DamageDose,
    ) -> Result<bool, &'static str> {
        if !dose.hp.is_finite() || dose.hp < 0.0 {
            return Err("invalid_domination_helo_damage_dose");
        }
        if flight == domination_helo::FlightState::Dormant
            || self.state == replication::VehicleState::Destroyed
            || self.wire.conditions & 12 != 0
            || dose.hp == 0.0
        {
            return Ok(false);
        }
        let old = self.hp;
        let hp = (old - dose.hp).max(profile.floor(flight)).min(old);
        if !self.set_hp(hp, profile.max_health, thresholds) {
            return Ok(false);
        }
        self.wire.last_attacker = dose.attacker;
        self.wire.last_location_and_type = dose.location_and_type;
        self.wire.last_source_object = dose.source_object;
        self.wire.last_user_data = dose.user_data;
        Ok(true)
    }

    fn set_hp(
        &mut self,
        hp: f32,
        max: f32,
        thresholds: structure_catalog::NativeStructureRules,
    ) -> bool {
        let hp = if hp < thresholds.dead_health_floor {
            0.0
        } else {
            hp
        };
        if hp == self.hp {
            return false;
        }
        self.hp = hp;
        self.wire.health_quarter_points = (hp * 4.0).trunc() as u16;
        self.wire.conditions = Self::conditions(hp, max, thresholds);
        true
    }

    pub(crate) fn tick(
        &mut self,
        profile: HeloDamageProfile,
        thresholds: structure_catalog::NativeStructureRules,
        dt: f32,
    ) -> Result<bool, &'static str> {
        if !dt.is_finite() || dt < 0.0 {
            return Err("invalid_domination_helo_damage_delta");
        }
        if self.wire.conditions & 0xc != 4 {
            return Ok(false);
        }
        let loss = ((profile.critical_loss_percent_per_second * 0.01) * profile.max_health) * dt;
        Ok(self.set_hp(self.hp - loss, profile.max_health, thresholds))
    }

    pub(crate) fn update_state(
        &mut self,
        profile: HeloDamageProfile,
        dt: f32,
    ) -> Result<HullTransition, &'static str> {
        use replication::VehicleState::{Critical, Destroyed};
        if !dt.is_finite() || dt < 0.0 {
            return Err("invalid_domination_helo_damage_delta");
        }
        if self.state == Destroyed {
            self.destroyed_age += dt;
            if self.wire.health_quarter_points == 0
                && self.destroyed_age > profile.destruction.effect_seconds
            {
                let mut reset = Self::fresh((profile.max_health * 4.0) as u16);
                reset.state = Destroyed;
                *self = reset;
                return Ok(HullTransition::Recovered);
            }
            return Ok(HullTransition::Waiting);
        }
        if self.wire.conditions & 8 != 0 {
            self.state = Destroyed;
            self.destroyed_age = 0.0;
            return Ok(HullTransition::Destroyed);
        }
        if self.wire.conditions & 4 != 0 && self.state != Critical {
            self.state = Critical;
            return Ok(HullTransition::Critical);
        }
        Ok(HullTransition::None)
    }

    pub(crate) fn wants_retreat(self, profile: HeloDamageProfile) -> bool {
        self.hp < (profile.max_health * profile.retreat_fraction) && self.wire.conditions & 0xc == 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HullTransition {
    None,
    Critical,
    Destroyed,
    Waiting,
    Recovered,
}

pub(crate) fn damage_delta(
    index: u16,
    state: Option<replication::VehicleState>,
    damage: Option<replication::VehicleDamage>,
    now: Duration,
    profile: crate::wire_profile::WireProfile,
) -> Result<Vec<u8>, &'static str> {
    use replication::{BitWriter, UpdateHeader};
    if index >= 8 || (state.is_none() && damage.is_none()) {
        return Err("invalid_domination_helo_damage_delta");
    }
    let mut bits = BitWriter::with_profile(profile);
    UpdateHeader {
        sequence: 2,
        nak_sequence: 0,
        client_index: 0,
        object_type_mask: 1 << 2,
        timestamp_ms: now.as_millis() as u32,
    }
    .encode(&mut bits)?;
    bits.write(1, 1)?;
    bits.write(index.into(), 16)?;
    for field in 0..27 {
        match (field, state, damage) {
            (5, Some(state), _) => {
                bits.write(1, 1)?;
                bits.write(state as u32, 8)?;
            }
            (18, _, Some(damage)) => {
                bits.write(1, 1)?;
                damage.encode(&mut bits)?;
            }
            _ => bits.write(0, 1)?,
        }
    }
    bits.write(0, 1)?;
    Ok(bits.finish())
}

pub(crate) struct PreparedHit {
    pub(crate) fleet: domination_helo::FleetRuntime,
    pub(crate) packet: Option<Vec<u8>>,
    pub(crate) audit: serde_json::Value,
}

impl<W: Write> Engine<W> {
    pub(crate) fn prepare_domination_helo_hit(
        &self,
        actor: &Active,
        request: &bullet_hit::NetMessageHit,
        now: Duration,
    ) -> Result<PreparedHit, &'static str> {
        let state = &self
            .domination_gameplay
            .as_ref()
            .ok_or("domination_helo_world_required")?
            .state;
        let rules = state
            .helo_damage
            .as_deref()
            .ok_or("domination_helo_damage_rules_required")?;
        let mut fleet = state.helo.clone().ok_or("domination_helo_world_required")?;
        let index = request.defender as u16;
        let carrier = fleet
            .carriers
            .iter_mut()
            .find(|carrier| carrier.source.object_index == index)
            .filter(|carrier| {
                request.defender == (0x0280_0000 | u32::from(carrier.source.object_index))
            })
            .ok_or("unknown_domination_helo_bullet_target")?;
        let profile = rules.profile(carrier.source.faction)?;
        let catalog = self
            .bunker_world
            .as_ref()
            .ok_or("domination_helo_armor_catalog_required")?
            .damage_catalog();
        let (weapon, damage) = self.retail_vehicle_bullet_parameters(actor, request)?;
        let friendly = if actor.team() == 0 {
            catalog.friendly_fire_multiplier()
        } else {
            1.0
        };
        let amount = item_catalog::apply_vehicle_armor(
            damage,
            request.penetration_fraction(),
            friendly,
            catalog.armor(profile.armor_type)?,
            catalog.body(request.hit_location)?,
        )?;
        let attacker = 0x0380_0000 | u32::from(actor.reservation.player);
        let before = carrier.damage.wire.health_quarter_points;
        let applied = carrier.damage.apply(
            carrier.published_state,
            profile,
            catalog.rules(),
            DamageDose {
                hp: amount,
                attacker,
                location_and_type: 0x10 | request.hit_location,
                source_object: u32::MAX,
                user_data: request.client_data,
            },
        )?;
        let packet = applied
            .then(|| {
                damage_delta(
                    index,
                    None,
                    Some(carrier.damage.wire),
                    now,
                    self.wire_profile,
                )
            })
            .transpose()?;
        let audit = serde_json::json!({
            "type": "domination_helo_bullet_hit", "target_id":request.defender,
            "attacker_id": attacker, "weapon_item":weapon, "amount_hp":amount,
            "health_before": before, "health_after":carrier.damage.wire.health_quarter_points,
            "client_data":request.client_data
        });
        Ok(PreparedHit {
            fleet,
            packet,
            audit,
        })
    }
}

#[derive(Clone, Copy)]
pub(crate) struct RadialDose {
    pub index: u16,
    pub world: u32,
    pub attacker: u32,
    pub damage: TargetDamageParameters,
    pub scale: f32,
    pub source_object: u32,
}

impl<W: Write> Engine<W> {
    pub(crate) fn stage_helo_radial_doses(
        &self,
        fleet: &mut domination_helo::FleetRuntime,
        rules: &HeloDamageRules,
        catalog: &structure_catalog::StructureCatalog,
        doses: &[RadialDose],
        now: Duration,
    ) -> Result<(Vec<Vec<u8>>, Vec<serde_json::Value>), &'static str> {
        let mut changed = BTreeMap::new();
        let mut audit = Vec::new();
        for dose in doses {
            let carrier = fleet
                .carriers
                .iter_mut()
                .find(|carrier| carrier.source.object_index == dose.index)
                .ok_or("domination_helo_radial_target_missing")?;
            let profile = rules.profile(carrier.source.faction)?;
            let friendly_factor =
                if self.character_team_in_world(dose.attacker, dose.world) == Some(0) {
                    catalog.friendly_fire_multiplier()
                } else {
                    1.0
                };
            let amount = item_catalog::apply_vehicle_armor(
                dose.damage,
                dose.scale,
                friendly_factor,
                catalog.armor(profile.armor_type)?,
                catalog.body(0)?,
            )?;
            if carrier.damage.apply(
                carrier.published_state,
                profile,
                catalog.rules(),
                DamageDose {
                    hp: amount,
                    attacker: dose.attacker,
                    location_and_type: 0xa0,
                    source_object: dose.source_object,
                    user_data: 0,
                },
            )? {
                changed.insert(dose.index, carrier.damage.wire);
                audit.push(serde_json::json!({
                    "op": "domination_helo_radial_damage", "index": dose.index,
                    "attacker": dose.attacker, "amount_hp": amount,
                    "health_quarter_points": carrier.damage.wire.health_quarter_points,
                }));
            }
        }
        let packets = changed
            .into_iter()
            .map(|(index, wire)| damage_delta(index, None, Some(wire), now, self.wire_profile))
            .collect::<Result<Vec<_>, _>>()?;
        Ok((packets, audit))
    }
}

pub(crate) struct PreparedGrenades {
    pub(crate) fleet: domination_helo::FleetRuntime,
    pub(crate) packets: Vec<Vec<u8>>,
    pub(crate) audit: Vec<serde_json::Value>,
}

impl<W: Write> Engine<W> {
    pub(crate) fn prepare_domination_helo_grenades(
        &self,
        actor: &Active,
        terminals: &[projectile::ProjectileTerminal],
        now: Duration,
    ) -> Result<Option<PreparedGrenades>, &'static str> {
        use domination_structural_effects::InstantaneousBlast;
        use domination_structural_world::{BlastCollision, BlastSource, BlastTarget, DamageWork};
        let Some(state) = self.domination_gameplay.as_ref().map(|world| &world.state) else {
            return Ok(None);
        };
        let Some(mut fleet) = state.helo.clone() else {
            return Ok(None);
        };
        let rules = state
            .helo_damage
            .as_deref()
            .ok_or("domination_helo_damage_rules_required")?;
        let catalog = self
            .bunker_world
            .as_ref()
            .ok_or("domination_helo_armor_catalog_required")?
            .damage_catalog();
        let player = u32::from(actor.reservation.player);
        let attacker = 0x0380_0000 | player;
        let mut doses = Vec::new();
        for terminal in terminals {
            let Some(position) = terminal.position.filter(|_| {
                terminal.reason == projectile::ProjectileTerminalReason::ReportedDetonation
            }) else {
                continue;
            };
            let metadata = self
                .item_catalog
                .retail_combat()?
                .grenade_for_damage(terminal.item_id)
                .map_err(|_| "unsupported_helo_grenade_item")?;
            let damage = TargetDamageParameters {
                amount: metadata.damage.vehicle as f32,
                armor_pierce: metadata.damage.vehicle_armor_pierce as f32,
                armor_redux: metadata.damage.vehicle_armor_reduction as f32,
            };
            let origin = [position.x, position.y, position.z];
            let work = self.collect_domination_blast(
                BlastSource::HeloGrenade,
                attacker,
                InstantaneousBlast {
                    origins: vec![origin],
                    inner_radius: metadata.half_radius as f32,
                    outer_radius: metadata.full_radius as f32,
                    damage: damage.amount,
                    material_penetration: rules.grenade_material_penetration,
                },
                BlastCollision {
                    filter_rows: &state.filter_rows,
                    limits: dynamic_collision::QueryLimits::default(),
                    sequences: &state.collisions,
                    material_costs: state.structural.material_penetration_costs(),
                    terrain_materials: Some(&state.terrain_materials),
                },
            )?;
            let DamageWork::Blast {
                blast,
                observations,
                ..
            } = work
            else {
                return Err("domination_helo_grenade_radial_work_required");
            };
            for observation in observations {
                let BlastTarget::Helo { index, world } = observation.target else {
                    return Err("invalid_domination_helo_grenade_target");
                };
                let distance = observation
                    .sample_position
                    .into_iter()
                    .zip(origin)
                    .map(|(a, b)| (a - b).powi(2))
                    .sum::<f32>()
                    .sqrt();
                doses.push(RadialDose {
                    index,
                    world,
                    attacker,
                    damage,
                    scale: blast.scale(distance)?,
                    source_object: ((16 + u32::from(terminal.slot)) << 24) | 0x0080_0000 | player,
                });
            }
        }
        if doses.is_empty() {
            return Ok(None);
        }
        let (packets, audit) =
            self.stage_helo_radial_doses(&mut fleet, rules, catalog, &doses, now)?;
        Ok(Some(PreparedGrenades {
            fleet,
            packets,
            audit,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domination_helo::FlightState;
    use replication::VehicleState;

    fn profile() -> HeloDamageProfile {
        HeloDamageProfile {
            max_health: 100.,
            armor_type: 2,
            critical_loss_percent_per_second: 10.,
            inbound_min_fraction: 0.,
            outbound_min_fraction: 0.,
            retreat_fraction: 0.4,
            destruction: Destruction {
                effect_seconds: 1.,
                damage: 50.,
                inner_radius: 2.,
                outer_radius: 3.,
                material_penetration: 1.0,
            },
        }
    }
    fn thresholds() -> structure_catalog::NativeStructureRules {
        structure_catalog::NativeStructureRules {
            light_percent: 70.,
            heavy_percent: 40.,
            critical_percent: 15.,
            dead_health_floor: 1.,
            min_rebuild_percent: 15.,
        }
    }
    fn dose(hp: f32) -> DamageDose {
        DamageDose {
            hp,
            attacker: 0x0380_0004,
            location_and_type: 0x12,
            source_object: u32::MAX,
            user_data: 83,
        }
    }

    #[test]
    fn helo_damage_defers_critical_and_death_to_sim_with_exact_dead_boundary() {
        let mut hull = HullDamage::fresh(400);
        let profile = profile();
        let thresholds = thresholds();
        assert!(
            !hull
                .apply(FlightState::Dormant, profile, thresholds, dose(30.))
                .unwrap()
        );
        assert!(
            hull.apply(FlightState::Hover, profile, thresholds, dose(62.))
                .unwrap()
        );
        assert!(hull.wants_retreat(profile));
        assert_eq!(hull.wire.health_quarter_points, 152);
        assert_eq!(hull.wire.conditions, 3);
        assert!(
            hull.apply(FlightState::Outbound, profile, thresholds, dose(24.))
                .unwrap()
        );
        assert!(!hull.wants_retreat(profile));
        assert_eq!(hull.state, VehicleState::Operational);
        assert_eq!(
            hull.update_state(profile, 0.).unwrap(),
            HullTransition::Critical
        );
        assert!(
            !hull
                .apply(FlightState::Outbound, profile, thresholds, dose(24.))
                .unwrap()
        );
        assert!(hull.tick(profile, thresholds, 1.35).unwrap());
        assert_eq!(hull.state, VehicleState::Critical);
        assert_eq!(hull.wire.last_user_data, 83);
        assert_eq!(hull.wire.health_quarter_points, 0);
        assert_eq!(
            hull.update_state(profile, 0.).unwrap(),
            HullTransition::Destroyed
        );
        assert_eq!(
            hull.update_state(profile, 1.).unwrap(),
            HullTransition::Waiting
        );
        assert_eq!(
            hull.update_state(profile, 0.01).unwrap(),
            HullTransition::Recovered
        );
        assert_eq!(hull.wire.health_quarter_points, 400);
        assert_eq!(hull.wire.last_attacker, u32::MAX);
        assert_eq!(hull.state, VehicleState::Destroyed);
        assert!(
            !hull
                .apply(FlightState::Outbound, profile, thresholds, dose(24.))
                .unwrap()
        );
        assert_eq!(
            hull.update_state(profile, 2.).unwrap(),
            HullTransition::Waiting
        );
    }

    #[test]
    fn helo_vehicle_damage_delta_emits_fields_five_and_eighteen_on_family_two() {
        use replication::{BitReader, ObjectUpdate};
        let profile = wire_profile::WireProfile::prototype();
        let damage = HullDamage::fresh(400).wire;
        let payload = damage_delta(
            2,
            Some(VehicleState::Critical),
            Some(damage),
            Duration::from_millis(500),
            profile,
        )
        .unwrap();
        let mut bits = BitReader::with_profile(&payload, profile);
        bits.position = 128 + profile.object_type_bits();
        let header = ObjectUpdate::decode_with_profile(&payload, profile)
            .unwrap()
            .header;
        assert_eq!(header.object_type_mask, 1 << 2);
        assert_eq!(header.timestamp_ms, 500);
        assert_eq!(bits.read(1).unwrap(), 1);
        assert_eq!(bits.read(16).unwrap(), 2);
        for field in 0..27 {
            let expected = [5, 18].contains(&field);
            assert_eq!(bits.read(1).unwrap() != 0, expected);
            match field {
                5 => assert_eq!(bits.read(8).unwrap(), 3),
                18 => {
                    assert_eq!(bits.read(8).unwrap(), 0);
                    assert_eq!(bits.read(16).unwrap(), 400);
                    assert_eq!(bits.read(32).unwrap(), u32::MAX);
                    assert_eq!(bits.read(32).unwrap(), u32::MAX);
                    assert_eq!(bits.read(32).unwrap(), 0);
                }
                _ => (),
            }
        }
        assert_eq!(bits.read(1).unwrap(), 0);
    }
}
