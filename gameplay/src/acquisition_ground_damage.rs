//! Source-bound vehicle armor, hit areas, and equipped weapon admission.
use crate::{
    acquisition_ground_runtime as ground, ground_vehicles, item_catalog::TargetDamageParameters, *,
};
use serde::Deserialize;
use std::collections::BTreeSet;

#[cfg(test)]
#[path = "mounted_ground_primary_tests.rs"]
mod mounted_primary_tests;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    friendly_fire_multiplier: f32,
    profiles: Vec<Profile>,
    areas: [Area; 16],
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    vehicle_type_token: u32,
    faction: u8,
    max_health: f32,
    critical_loss_percent_per_second: f32,
    armor: Armor,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Armor {
    rating: f32,
    prevention: f32,
    coverage: [f32; 4],
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Area {
    armor_area: usize,
    multiplier: f32,
}

impl Policy {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        let mut variants = BTreeSet::new();
        if !(0.0..=1.0).contains(&self.friendly_fire_multiplier)
            || self.profiles.is_empty()
            || self.profiles.len() > 32
            || self.profiles.iter().any(|profile| {
                profile.vehicle_type_token == 0
                    || profile.faction > 2
                    || !variants.insert((profile.vehicle_type_token, profile.faction))
                    || !(1.0..=1023.75).contains(&profile.max_health)
                    || !(0.0..=100.0).contains(&profile.critical_loss_percent_per_second)
                    || !profile.armor.rating.is_finite()
                    || profile.armor.rating < 0.0
                    || !(0.0..=1.0).contains(&profile.armor.prevention)
                    || profile
                        .armor
                        .coverage
                        .iter()
                        .any(|v| !(0.0..=1.0).contains(v))
            })
            || self.areas.iter().any(|area| {
                area.armor_area >= 4 || !area.multiplier.is_finite() || area.multiplier < 0.0
            })
        {
            return Err("invalid_acquisition_ground_damage_policy");
        }
        Ok(())
    }

    fn profile(&self, token: u32, faction: u8) -> Result<&Profile, &'static str> {
        self.profiles
            .iter()
            .find(|profile| profile.vehicle_type_token == token && profile.faction == faction)
            .ok_or("acquisition_ground_damage_profile_unavailable")
    }

    pub(crate) fn validate_vehicle(
        &self,
        creation: &ground_vehicles::Creation,
    ) -> Result<(), &'static str> {
        let profile = self.profile(creation.vehicle_type_token, creation.faction)?;
        if profile.max_health * 4.0 != f32::from(creation.health_quarter_points) {
            return Err("acquisition_ground_damage_health_mismatch");
        }
        Ok(())
    }

    pub(crate) fn advance_critical(
        &self,
        runtime: &mut ground::Runtime,
        key: ground::Key,
        dt: f32,
    ) -> Result<Vec<ground::LifecycleEvent>, &'static str> {
        let vehicle = runtime
            .get(key.index)
            .filter(|vehicle| vehicle.key == key)
            .ok_or("stale_acquisition_ground_candidate")?;
        if !vehicle.is_dying() {
            return Ok(vec![]);
        }
        let profile = self.profile(vehicle.vehicle_type_token, vehicle.faction)?;
        runtime.critical_decay(key, profile.critical_loss_percent_per_second, dt)
    }

    pub(crate) fn blast_amount(
        &self,
        creation: &ground_vehicles::Creation,
        damage: TargetDamageParameters,
        radial_scale: f32,
        friendly: bool,
    ) -> Result<f32, &'static str> {
        // The native radius callback uses damage-info indices 3/4/5 and
        // vehicle area 0. Destruction constructs those reductions as zero;
        // authored equipment explosions carry their own reductions.
        self.bullet_amount(
            creation.vehicle_type_token,
            creation.faction,
            damage,
            radial_scale,
            0,
            friendly,
        )
    }

    pub(crate) fn classified_vehicle_blast_amount(
        &self,
        creation: &ground_vehicles::Creation,
        raw_hp: f32,
        radial_scale: f32,
        location: u8,
        friendly: bool,
    ) -> Result<f32, &'static str> {
        self.classified_equipment_blast_amount(
            creation,
            TargetDamageParameters {
                amount: raw_hp,
                armor_pierce: 0.0,
                armor_redux: 0.0,
            },
            radial_scale,
            location,
            friendly,
        )
    }

    pub(crate) fn classified_equipment_blast_amount(
        &self,
        creation: &ground_vehicles::Creation,
        damage: TargetDamageParameters,
        radial_scale: f32,
        location: u8,
        friendly: bool,
    ) -> Result<f32, &'static str> {
        if !(1..=6).contains(&location) {
            return Err("unsupported_vehicle_hit_location");
        }
        self.bullet_amount(
            creation.vehicle_type_token,
            creation.faction,
            damage,
            radial_scale,
            location,
            friendly,
        )
    }

    fn bullet_amount(
        &self,
        token: u32,
        faction: u8,
        damage: TargetDamageParameters,
        penetration: f32,
        location: u8,
        friendly: bool,
    ) -> Result<f32, &'static str> {
        if ![
            damage.amount,
            damage.armor_pierce,
            damage.armor_redux,
            penetration,
        ]
        .into_iter()
        .all(|value| value.is_finite() && value >= 0.0)
        {
            return Err("invalid_acquisition_ground_damage_input");
        }
        let armor = self.profile(token, faction)?.armor;
        let area = self
            .areas
            .get(usize::from(location))
            .ok_or("unsupported_vehicle_hit_location")?;
        let raw = damage.amount * penetration;
        let scaled = raw
            * if friendly {
                self.friendly_fire_multiplier
            } else {
                1.0
            };
        let rating = (armor.rating - damage.armor_pierce).max(0.0);
        let remaining = (scaled - rating).max(0.0);
        let prevention = (armor.prevention - damage.armor_redux).max(0.0);
        let passed = 1.0 - armor.coverage[area.armor_area] * prevention;
        let applied = remaining * (area.multiplier * passed);
        if !applied.is_finite() {
            return Err("invalid_acquisition_ground_damage_amount");
        }
        Ok(applied.max(0.0))
    }
}

pub(crate) struct PreparedHit {
    pub(crate) after: ground::Runtime,
    pub(crate) packet: Option<Vec<u8>>,
    pub(crate) audit: Value,
}

impl<W: Write> Engine<W> {
    pub(crate) fn ground_damage_policy(&self) -> Option<&Policy> {
        if let Some(world) = &self.domination_gameplay {
            return Some(&world.state.ground_damage);
        }
        self.acquisition_gameplay
            .as_ref()
            .and_then(|gameplay| gameplay.setup.ground_damage.as_ref())
    }

    pub(crate) fn prepare_acquisition_ground_hit(
        &self,
        actor: &Active,
        request: &bullet_hit::NetMessageHit,
        now: Duration,
    ) -> Result<PreparedHit, &'static str> {
        let policy = self
            .ground_damage_policy()
            .ok_or("acquisition_ground_damage_unconfigured")?;
        let ground_gunner = self.ground_gunner_accepts_ground_target(actor);
        // A qualified hospital primary reaches the same Ground targets as an
        // on-foot firearm in either mode; its authority is checked when the
        // mounted profile resolves.
        let mounted_primary = ground_gunner || actor.is_indexed_hospital_gunner();
        if !actor.entered_game
            || !actor.ready_for_updates
            || !actor.life.is_alive()
            || (actor.is_mounted() && !mounted_primary)
        {
            return Err("ground_hit_requires_living_onfoot_actor");
        }
        let index = request.defender as u16;
        let catalog = self
            .ground_vehicles
            .as_ref()
            .ok_or("ground_vehicle_catalog_required")?;
        let creation = catalog
            .vehicle(index)
            .filter(|vehicle| vehicle.object_id() == request.defender)
            .ok_or("unknown_ground_bullet_target")?;
        if catalog.vehicle_source(index).is_none_or(|source| {
            if self.domination_gameplay.is_some() {
                !matches!(source.spawn_type, 3 | 7)
            } else {
                source.spawn_type != 9 && (source.spawn_type != 3 || ground_gunner)
            }
        }) {
            return Err("unsupported_ground_bullet_target");
        }
        let before = self
            .ground_runtime
            .as_ref()
            .ok_or("ground_vehicle_runtime_required")?;
        let vehicle = before.get(index).ok_or("unknown_ground_bullet_target")?;
        if vehicle.state != 2 || vehicle.body != ground::Body::Operational || vehicle.is_dead() {
            return Err("ground_bullet_target_not_operational");
        }
        if vehicle.is_dying() {
            return Err("ground_bullet_target_is_dying");
        }
        let profile = if mounted_primary {
            self.mounted_primary_profile(actor, request)?
        } else {
            self.bullet_profile(actor, request)?
        };
        let (CharacterHitProfile::Retail(profile)
        | CharacterHitProfile::HospitalPrimary(profile)
        | CharacterHitProfile::GroundApcPrimary(profile)
        | CharacterHitProfile::GroundEscortPrimary(profile)) = profile
        else {
            return Err("unsupported_ground_bullet_weapon");
        };
        let amount = policy.bullet_amount(
            creation.vehicle_type_token,
            creation.faction,
            profile.vehicle_damage(),
            request.penetration_fraction(),
            request.hit_location,
            actor.team() == creation.team,
        )?;
        let mut after = before.clone();
        let attacker = 0x0380_0000 | u32::from(actor.reservation.player);
        let changes = after.apply_bullet_damage(
            vehicle.key,
            amount,
            attacker,
            request.hit_location,
            request.client_data,
        )?;
        let packet = if changes.is_empty() {
            None
        } else {
            self.encode_ground_changes(&after, &[(index, 1 << 18)].into(), now)?
        };
        let audit = json!({
            "type":"ground_bullet_hit","target_id":request.defender,"attacker_id":attacker,
            "weapon_item":profile.item_id(),"penetration_milli":request.penetration_milli,
            "location":request.hit_location,"client_data":request.client_data,
            "amount_hp":amount,"health_before":vehicle.health_quarter_points,
            "health_after":after.get(index).unwrap().health_quarter_points,
        });
        Ok(PreparedHit {
            after,
            packet,
            audit,
        })
    }
}

impl<W: Write> Engine<W> {
    pub(crate) fn stage_ground_grenade_damage(
        &self,
        runtime: &mut ground::Runtime,
        actor: &Active,
        terminals: &[projectile::ProjectileTerminal],
        now: Duration,
    ) -> Result<(Option<Vec<u8>>, Vec<Value>), &'static str> {
        use crate::{
            acquisition_volume_damage::TargetKind,
            dynamic_collision::{Geometry, Pose, geometry},
            projectile::ProjectileTerminalReason,
        };
        let Some(policy) = self.ground_damage_policy() else {
            return Ok((None, vec![]));
        };
        let (filter_row, limits) = if let Some(world) = &self.domination_gameplay {
            (world.state.filter_rows[6], Default::default())
        } else {
            let setup = &self
                .acquisition_gameplay
                .as_ref()
                .ok_or("acquisition_ground_damage_unconfigured")?
                .setup;
            (setup.filter_rows[6], setup.physics.query_limits)
        };
        let snapshot = self.dynamic_collision_snapshot(filter_row)?;
        let mut budget = limits.max_support_points;
        let mut changes = std::collections::BTreeMap::new();
        let mut audit = Vec::new();
        let player = u32::from(actor.reservation.player);
        let attacker = 0x0380_0000 | player;
        for terminal in terminals {
            let Some(origin) = terminal
                .position
                .filter(|_| terminal.reason == ProjectileTerminalReason::ReportedDetonation)
            else {
                continue;
            };
            let source = ((16 + u32::from(terminal.slot)) << 24) | 0x0080_0000 | player;
            let metadata = self
                .item_catalog
                .retail_combat()?
                .grenade_for_damage(terminal.item_id)
                .map_err(|_| "unsupported_ground_grenade_item")?;
            let damage = TargetDamageParameters {
                amount: metadata.damage.vehicle as f32,
                armor_pierce: metadata.damage.vehicle_armor_pierce as f32,
                armor_redux: metadata.damage.vehicle_armor_reduction as f32,
            };
            if damage.amount == 0.0 {
                continue;
            }
            let profile = armor::GrenadeProfile::from_item_id_with_catalog(
                terminal.item_id,
                &self.item_catalog,
            )?;
            let radius = Geometry::Sphere {
                radius: metadata.full_radius,
            };
            let pose = Pose::at([origin.x, origin.y, origin.z].map(f64::from));
            let mut seen = BTreeSet::new();
            // Geometry comes from committed poses. This damage remains
            // authoritative even if a same-report driver input is rejected.
            for body in snapshot.bodies() {
                if body.target.kind != TargetKind::GroundVehicle
                    || filter_row & (1 << body.layer) == 0
                    || seen.contains(&body.target.key.id)
                    || !geometry::penetrates(
                        &radius,
                        pose,
                        &body.shape,
                        body.pose,
                        limits,
                        &mut budget,
                    )?
                {
                    continue;
                }
                seen.insert(body.target.key.id);
                let index = body.target.key.id as u16;
                let key = ground::Key {
                    index,
                    generation: body.target.key.generation,
                };
                if !runtime.get(index).is_some_and(|vehicle| {
                    vehicle.key == key
                        && vehicle.state == 2
                        && !vehicle.is_dead()
                        && !vehicle.is_dying()
                }) {
                    continue;
                }
                let creation = self
                    .current_ground_vehicle(index)
                    .ok_or("unknown_ground_vehicle")?;
                let point = creation.position;
                let distance = ((point.x - origin.x).powi(2)
                    + (point.y - origin.y).powi(2)
                    + (point.z - origin.z).powi(2))
                .sqrt();
                let scale = profile.radius_scale(distance)?;
                if scale == 0.0 {
                    continue;
                }
                let visibility = blast_visibility::query_blast_visibility(
                    self.collision_world.as_ref(),
                    self.map_name(),
                    origin,
                    point,
                );
                let amount =
                    policy.blast_amount(&creation, damage, scale, actor.team() == creation.team)?
                        * visibility.damage_scale().unwrap_or(0.0);
                if !runtime
                    .apply_grenade_damage(key, amount, attacker, source)?
                    .is_empty()
                {
                    changes.insert(index, 1 << 18);
                }
                audit.push(json!({"vehicle_index":index,"slot":terminal.slot,"generation":terminal.generation,
                    "amount_hp":amount,"source_object":source,"visibility":visibility}));
            }
        }
        Ok((self.encode_ground_changes(runtime, &changes, now)?, audit))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rocket_uses_classified_vehicle_area_and_friendly_fire_before_armor() {
        let mut areas = vec![serde_json::json!({"armor_area": 0, "multiplier": 1.0}); 16];
        areas[5] = serde_json::json!({"armor_area": 2, "multiplier": 2.0});
        let policy: Policy = serde_json::from_value(serde_json::json!({
            "friendly_fire_multiplier": 0.5,
            "profiles": [{
                "vehicle_type_token": 19,
                "faction": 1,
                "max_health": 200.0,
                "critical_loss_percent_per_second": 3.0,
                "armor": {"rating": 10.0, "prevention": 0.5, "coverage": [0.0, 0.0, 0.5, 0.0]}
            }],
            "areas": areas
        }))
        .unwrap();
        let creation: ground_vehicles::Creation = serde_json::from_value(serde_json::json!({
            "index": 4, "spawn_point_index": 12, "spawn_point_token": 99,
            "vehicle_type_token": 19, "faction": 1, "team": 0, "modifier": 0,
            "position": {"x": 2.0, "y": 4.0, "z": 8.0},
            "rotation": [0.0, 0.0, 0.0, 1.0],
            "health_quarter_points": 800, "initial_state": 2,
        }))
        .unwrap();
        assert_eq!(
            policy.classified_vehicle_blast_amount(&creation, 100.0, 0.5, 5, false),
            Ok(60.0)
        );
        assert_eq!(
            policy.classified_vehicle_blast_amount(&creation, 100.0, 0.5, 5, true),
            Ok(22.5)
        );
        assert!(
            policy
                .classified_vehicle_blast_amount(&creation, 100.0, 0.5, 0, false)
                .is_err()
        );
        assert_eq!(
            policy.classified_equipment_blast_amount(
                &creation,
                TargetDamageParameters {
                    amount: 100.,
                    armor_pierce: 0.,
                    armor_redux: 0.1,
                },
                0.5,
                5,
                false,
            ),
            Ok(64.)
        );
        assert_eq!(
            policy.classified_equipment_blast_amount(
                &creation,
                TargetDamageParameters {
                    amount: 100.,
                    armor_pierce: 10.,
                    armor_redux: 0.25,
                },
                0.5,
                5,
                true,
            ),
            Ok(43.75)
        );
    }

    #[test]
    fn vehicle_hit_applies_friendly_fire_before_armor_without_character_range() {
        let policy: Policy = serde_json::from_value(serde_json::json!({
            "friendly_fire_multiplier": 0.5,
            "profiles": [{
                "vehicle_type_token": 19,
                "faction": 1,
                "max_health": 200.0,
                "critical_loss_percent_per_second": 3.0,
                "armor": {"rating": 10.0, "prevention": 0.5, "coverage": [0.5, 0.5, 0.5, 0.5]}
            }],
            "areas": vec![serde_json::json!({"armor_area": 2, "multiplier": 2.0}); 16]
        }))
        .unwrap();
        let damage = crate::item_catalog::TargetDamageParameters {
            amount: 100.0,
            armor_pierce: 5.0,
            armor_redux: 0.25,
        };
        assert_eq!(policy.bullet_amount(19, 1, damage, 0.5, 0, true), Ok(35.0));
        assert_eq!(
            policy.bullet_amount(19, 1, damage, 0.5, 0, false),
            Ok(78.75)
        );
        assert!(policy.bullet_amount(19, 2, damage, 0.5, 0, true).is_err());
        assert!(policy.bullet_amount(19, 1, damage, 0.5, 16, true).is_err());
    }
}
