//! Operational Helo effects from an authoritative hospital rocket's vehicle channel.
use super::*;
use crate::{
    domination_helo::FleetRuntime,
    domination_helo_damage::{DamageDose, damage_delta},
    hospital_rocket_helo::Catalog,
    hospital_rocket_runtime::LaunchSource,
    hospital_rockets::Blast,
    item_catalog::TargetDamageParameters,
};

#[derive(Clone, Copy)]
pub(crate) struct HeloRocketDose {
    vehicle_hp: f32,
    half_radius_m: f32,
    outer_radius_m: f32,
    material_penetration: f32,
}

impl From<Blast> for HeloRocketDose {
    fn from(blast: Blast) -> Self {
        Self {
            vehicle_hp: blast.vehicle_hp,
            half_radius_m: blast.half_radius_m,
            outer_radius_m: blast.outer_radius_m,
            material_penetration: blast.material_penetration,
        }
    }
}

#[derive(Default)]
pub(crate) struct HeloRocketCandidate {
    pub(crate) after: Option<FleetRuntime>,
    pub(crate) packets: Vec<Vec<u8>>,
    pub(crate) audit: Vec<Value>,
}

impl<W: Write> Engine<W> {
    /// Clone only the qualified current fleet. The caller owns terminal
    /// publication and installs this candidate after its record succeeds.
    pub(crate) fn prepare_hospital_rocket_helo_blast(
        &self,
        source: &LaunchSource,
        origin: [f64; 3],
        dose: HeloRocketDose,
        rules: &Catalog,
        collision: BlastCollision<'_>,
        now: Duration,
    ) -> Result<HeloRocketCandidate, &'static str> {
        let attacker = self.hospital_rocket_blast_attacker(source)?;
        let origin_f32 = origin.map(|value| value as f32);
        if !origin_f32.into_iter().all(f32::is_finite)
            || !dose.vehicle_hp.is_finite()
            || dose.vehicle_hp <= 0.
            || !dose.half_radius_m.is_finite()
            || !dose.outer_radius_m.is_finite()
            || dose.half_radius_m <= 0.
            || dose.outer_radius_m <= dose.half_radius_m
            || !dose.material_penetration.is_finite()
            || dose.material_penetration < 0.
        {
            return Err("invalid_hospital_rocket_helo_blast");
        }
        let state = &self
            .domination_gameplay
            .as_ref()
            .ok_or("hospital_rocket_domination_world_required")?
            .state;
        let fleet = state
            .helo
            .as_ref()
            .ok_or("domination_helo_world_required")?;
        let mut candidate = HeloRocketCandidate::default();
        if collision.filter_rows[6] & (1 << 22) == 0
            || !fleet
                .carriers
                .iter()
                .any(|carrier| carrier.damage.accepts_damage(carrier.published_state))
        {
            return Ok(candidate);
        }
        let assets = state
            .helo_collision
            .as_deref()
            .ok_or("hospital_rocket_helo_collision_required")?;
        let damage = state
            .helo_damage
            .as_deref()
            .ok_or("domination_helo_damage_rules_required")?;
        let armor = self
            .bunker_world
            .as_ref()
            .ok_or("domination_helo_armor_catalog_required")?
            .damage_catalog();
        let snapshot = self.dynamic_collision_snapshot(collision.filter_rows[6])?;
        if snapshot.context().0.world_id != source.world {
            return Err("stale_hospital_rocket_helo_world");
        }
        let scene = self
            .collision_world
            .as_ref()
            .filter(|scene| {
                scene.stats().triangles != 0 && scene.metadata().map_name == self.map_name()
            })
            .ok_or("hospital_rocket_helo_static_collision_required")?;
        let sphere = Geometry::Sphere {
            radius: f64::from(dose.outer_radius_m),
        };
        let mut overlap_budget = collision.limits.max_support_points;
        let mut static_budget = 1_000_000;
        let friendly = source.team == 0; // Helo belongs to the attacking team.
        let friendly_factor = if friendly {
            armor.friendly_fire_multiplier()
        } else {
            1.
        };
        let mut after = fleet.clone();

        for carrier in &mut after.carriers {
            if !carrier.damage.accepts_damage(carrier.published_state) {
                continue;
            }
            let index = carrier.source.object_index;
            let faction = carrier.source.faction;
            let surface = collision
                .sequences
                .helos
                .get(&index)
                .ok_or("hospital_rocket_helo_mesh_required")?;
            let live_position = carrier
                .current_pose
                .ok_or("hospital_rocket_helo_pose_required")?
                .position;
            let live_pose = carrier
                .live_basis
                .ok_or("hospital_rocket_helo_pose_required")?
                .place(live_position)?;
            let mesh = assets
                .by_faction
                .get(&faction)
                .ok_or("unbound_domination_helo_faction_mesh")?;
            if surface.pose.translation != live_pose.translation
                || surface.pose.rotation != live_pose.rotation
                || surface.mesh.source_sha256() != mesh.source_sha256()
            {
                return Err("hospital_rocket_helo_collision_mismatch");
            }
            if !surface.overlaps_sphere(
                surface.local(origin),
                &sphere,
                collision.limits,
                &mut overlap_budget,
            )? {
                continue;
            }
            let anchor = surface.pose.translation;
            let (displacement, distance_squared) =
                separation(origin_f32, anchor.map(|axis| axis as f32))?;
            let scale = armor::native_radial_scale(
                distance_squared.sqrt(),
                dose.half_radius_m,
                dose.outer_radius_m,
            )?;
            let target = 0x0280_0000 | u32::from(index);
            if scale <= 0.
                || !collision.clear_path(
                    scene,
                    &snapshot,
                    BlastRay {
                        start: origin,
                        end: anchor,
                        excludes: [Some(u64::from(target)), None],
                        penetration: dose.material_penetration,
                        zero_penetration_strike: false,
                        filter_static: true,
                    },
                    &mut static_budget,
                )?
            {
                continue;
            }
            let location = rules.location(faction, live_pose, origin)?;
            let profile = damage.profile(faction)?;
            let amount = item_catalog::apply_vehicle_armor(
                TargetDamageParameters {
                    amount: dose.vehicle_hp,
                    armor_pierce: 0.,
                    armor_redux: 0.,
                },
                scale,
                friendly_factor,
                armor.armor(profile.armor_type)?,
                armor.body(location)?,
            )?;
            let user_data = character::explosion_direction_user_data(direction(
                displacement,
                distance_squared,
            ))?;
            let before = carrier.damage.wire.health_quarter_points;
            if !carrier.damage.apply(
                carrier.published_state,
                profile,
                armor.rules(),
                DamageDose {
                    hp: amount,
                    attacker,
                    location_and_type: 0xa0 | location,
                    source_object: u32::MAX,
                    user_data,
                },
            )? {
                continue;
            }
            let wire = carrier.damage.wire;
            candidate.packets.push(damage_delta(
                index,
                None,
                Some(wire),
                now,
                self.wire_profile,
            )?);
            candidate.audit.push(json!({
                "vehicle_index": index, "target": target, "location": location,
                "projectile_id": source.projectile_id, "attacker": attacker,
                "friendly": friendly, "radial_scale": scale, "amount_hp": amount,
                "user_data": user_data, "health_before_quarter_points": before,
                "health_after_quarter_points": wire.health_quarter_points,
            }));
        }
        if !candidate.packets.is_empty() {
            candidate.after = Some(after);
        }
        Ok(candidate)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        domination_helo::{FleetSource, FlightPose, FlightState},
        domination_helo_damage::HeloDamageRules,
        domination_helo_engine::BodyBasis,
        replication::{BitReader, CharacterPosition, ObjectUpdate, VehicleState},
    };

    const ORIGIN: [f64; 3] = [3., 2.5, 1.];
    const POSE: CharacterPosition = CharacterPosition {
        x: 2.,
        y: 2.5,
        z: 1.,
    };
    const BASIS: BodyBasis = BodyBasis {
        right: [1., 0., 0.],
        forward: [0., 0., 1.],
    };
    const DOSE: Blast = Blast {
        character_hp: 1.,
        vehicle_hp: 80.,
        half_radius_m: 2.,
        outer_radius_m: 5.,
        material_penetration: 1.25,
        objective_hp: 1.,
    };

    fn fleet_source() -> FleetSource {
        let knots = [0., 0.5, 1.].map(|t| {
            json!({
                "position_xyz":[10.,10.,t*20.],"normalized_parameter":t,"tangent_xyz":[0.,0.,20.]
            })
        });
        let carriers: Vec<_> = (0..4).map(|index| json!({
            "object_index":index,"spawn_index":index,"spawn_token":1+index,"vehicle_type_token":1,
            "health_quarter_points":400,"faction":0,"passenger_start":1,"squads":[0,1],
            "start_delay_max_seconds":0.,"acceleration_seconds":1.,"braking_seconds":1.,
            "route":{"inbound_flight_seconds":5.,"outbound_flight_seconds":5.,
                "hover_position_delay_seconds":1.,"drop_point_index":1,"knots":knots}
        })).collect();
        serde_json::from_value(json!({"schema":"mag_domination_helo_fleet_v1","map":"synthetic",
            "spawn_catalog_sha256":"a".repeat(64),"source_receipt_sha256":vec!["a".repeat(64);3],
            "carriers":carriers
        })).unwrap()
    }

    pub(crate) fn install_helo_fixture(engine: &mut Engine<Vec<u8>>, position: CharacterPosition) {
        install_helo_with_mesh_offset(engine, position, 0.5);
    }

    fn install_helo_with_mesh_offset(
        engine: &mut Engine<Vec<u8>>,
        position: CharacterPosition,
        offset: f32,
    ) {
        let source = fleet_source();
        let rules = HeloDamageRules::from_json(&serde_json::to_vec(
            &json!({"schema":"mag_domination_helo_damage_v1",
                "retail_elf_sha256":"a".repeat(64),"models_sha256":"a".repeat(64),
                "grenade_material_penetration":0.5,"profiles":(0..3).map(|faction|(
                    faction.to_string(), json!({
                        "max_health":100.,"armor_type":1,"critical_loss_percent_per_second":10.,
                        "inbound_min_fraction":0.9,"outbound_min_fraction":0.8,"retreat_fraction":0.5,
                        "destruction":{"effect_seconds":1.,"damage":10.,"inner_radius":1.,
                            "outer_radius":2.,"material_penetration":0.5}
                    }))).collect::<BTreeMap<_,_>>()
            })
        ).unwrap(), &"a".repeat(64), &source).unwrap();
        let mut fleet = FleetRuntime::from_source(source, 5).unwrap();
        let carrier = &mut fleet.carriers[0];
        carrier.current_pose = Some(FlightPose {
            position,
            forward: BASIS.forward,
        });
        carrier.live_basis = Some(BASIS);
        carrier.published_state = FlightState::Hover;

        // This chassis intersects the victim's own material ray from +X.
        let mesh = collision::CollisionWorld::from_bytes(
            &serde_json::to_vec(&json!({
                "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
                "metadata":{"map_name":engine.map_name(),"build":"synthetic",
                    "coverage":"provisional","provenance":["synthetic"],"unknowns":[]},
                "instances":[{"id":"hull","material_index":255,
                    "vertices_xyz":[[offset,-1.,-1.],[offset,1.,-1.],[offset,0.,1.]],
                    "triangles":[[0,1,2]]}]
            }))
            .unwrap(),
            collision::CollisionLimits::default(),
        )
        .unwrap();
        let assets = HeloCollisionAssets {
            projectile: BTreeMap::new(),
            by_faction: BTreeMap::from([(0, mesh)]),
        };
        let state = &mut engine.domination_gameplay.as_mut().unwrap().state;
        state.filter_rows[6] |= 1 << 22;
        state.filter_rows[24] = (1 << 22) | (1 << 9) | 1;
        state
            .collisions
            .place_helo(0, 0, BASIS.place(position).unwrap(), &assets)
            .unwrap();
        state.helo_collision = Some(std::sync::Arc::new(assets));
        state.helo_damage = Some(std::sync::Arc::new(rules));
        state.helo = Some(fleet);
    }

    fn fixture() -> (Engine<Vec<u8>>, LaunchSource) {
        let mut engine = crate::hospital_rocket_runtime::tests::tick_fixture();
        install_helo_fixture(&mut engine, POSE);
        let actor = engine.active.as_ref().unwrap();
        let source = LaunchSource {
            reservation: actor.reservation.id.clone(),
            world: actor.reservation.world,
            round: engine.round_number,
            match_id: actor.reward_match_id.clone(),
            character_id: actor.reservation.character.character_id,
            player: actor.reservation.player,
            life_generation: actor.life.damage_health().unwrap().0,
            team: actor.team(),
            item: 42,
            projectile_id: 0x1380_0000,
        };
        (engine, source)
    }

    fn candidate(
        engine: &Engine<Vec<u8>>,
        source: &LaunchSource,
    ) -> Result<HeloRocketCandidate, &'static str> {
        let state = &engine.domination_gameplay.as_ref().unwrap().state;
        engine.prepare_hospital_rocket_helo_blast(
            source,
            ORIGIN,
            DOSE.into(),
            &Catalog::synthetic(),
            BlastCollision {
                filter_rows: &state.filter_rows,
                limits: Default::default(),
                sequences: &state.collisions,
                material_costs: state.structural.material_penetration_costs(),
                terrain_materials: Some(&state.terrain_materials),
            },
            Duration::from_millis(200),
        )
    }

    #[test]
    fn rocket_helo_current_mesh_applies_classified_vehicle_damage_and_field18() {
        let (engine, source) = fixture();
        assert_eq!(source.team, 1);
        let candidate = candidate(&engine, &source).unwrap();
        let initial = engine
            .domination_gameplay
            .as_ref()
            .unwrap()
            .state
            .helo
            .as_ref()
            .unwrap();
        assert_eq!(initial.carriers[0].damage.wire.health_quarter_points, 400);
        let after = candidate.after.as_ref().expect("qualified Helo hit");
        let damage = after.carriers[0].damage;
        assert_eq!(damage.state, VehicleState::Operational);
        // 80 HP × 0.75 radial scale minus 6 rating, then × 0.5 coverage.
        assert_eq!(damage.wire.health_quarter_points, 292);
        assert_eq!(damage.wire.last_location_and_type, 0xa2);
        assert_eq!(damage.wire.last_attacker, 0x0380_0000);
        assert_eq!(damage.wire.last_source_object, u32::MAX);
        assert_eq!(damage.wire.last_user_data, (128 | 512) << 20);
        assert_eq!(candidate.audit[0]["projectile_id"], source.projectile_id);
        let [packet] = candidate.packets.as_slice() else {
            panic!("one field18 frame expected");
        };
        assert_eq!(
            ObjectUpdate::decode_with_profile(packet, engine.wire_profile)
                .unwrap()
                .header
                .object_type_mask,
            1 << 2
        );
        let mut bits = BitReader::with_profile(packet, engine.wire_profile);
        bits.position = engine.wire_profile.header_bits();
        assert_eq!(bits.read(1), Ok(1));
        assert_eq!(bits.read(16), Ok(0));
        assert_eq!(bits.read(18), Ok(0)); // Includes state field5.
        assert_eq!(bits.read(1), Ok(1));
        assert_eq!(bits.read(8), Ok(0xa2));
        assert_eq!(bits.read(16), Ok(292));
        assert_eq!(bits.read(32), Ok(0x0380_0000));
        assert_eq!(bits.read(32), Ok(u32::MAX));
        assert_eq!(bits.read(32), Ok((128 | 512) << 20));
        assert_eq!(bits.read(9), Ok(0)); // Remaining fields and list terminator.
    }

    #[test]
    fn rocket_helo_requires_both_chassis_overlap_and_positive_entity_origin_scale() {
        let (mut engine, source) = fixture();
        install_helo_with_mesh_offset(&mut engine, POSE, 7.);
        assert!(candidate(&engine, &source).unwrap().after.is_none());
        install_helo_with_mesh_offset(&mut engine, CharacterPosition { x: 8.5, ..POSE }, -1.);
        assert!(candidate(&engine, &source).unwrap().after.is_none());
    }

    #[test]
    fn rocket_helo_material_cover_uses_the_world_but_excludes_the_target_chassis() {
        let (mut engine, source) = fixture();
        engine.collision_world = Some(collision::CollisionWorld::from_bytes(
            &serde_json::to_vec(&json!({
                "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
                "metadata":{"map_name":engine.map_name(),"build":"synthetic","coverage":"provisional",
                    "provenance":["synthetic"],"unknowns":[]},
                "instances":[{"id":"blocking-wall","material_index":255,
                    "vertices_xyz":[[2.75,1.,0.],[2.75,4.,0.],[2.75,2.5,2.]],
                    "triangles":[[0,1,2]],
                    "collision":{"filter_info":9,"ray_child_filter_info":0,"convex_radius":0.,
                        "provenance":"synthetic","child_filter_policy":"uniform"}}]
            })).unwrap(), collision::CollisionLimits::default(),
        ).unwrap());
        let blocked = candidate(&engine, &source).unwrap();
        assert!(blocked.after.is_none());
        assert!(blocked.packets.is_empty());
    }

    #[test]
    fn rocket_helo_checks_operational_geometry_without_requiring_dormant_or_dead_meshes() {
        let (mut engine, source) = fixture();
        let state = &mut engine.domination_gameplay.as_mut().unwrap().state;
        state.collisions.helos.get_mut(&0).unwrap().pose.translation[0] += 1.;
        assert!(matches!(
            candidate(&engine, &source),
            Err("hospital_rocket_helo_collision_mismatch")
        ));
        let state = &mut engine.domination_gameplay.as_mut().unwrap().state;
        state.collisions.remove_helo(0);
        assert!(matches!(
            candidate(&engine, &source),
            Err("hospital_rocket_helo_mesh_required")
        ));
        let fleet = engine
            .domination_gameplay
            .as_mut()
            .unwrap()
            .state
            .helo
            .as_mut()
            .unwrap();
        fleet.carriers[0].published_state = FlightState::Dormant;
        assert!(candidate(&engine, &source).unwrap().after.is_none());
        let fleet = engine
            .domination_gameplay
            .as_mut()
            .unwrap()
            .state
            .helo
            .as_mut()
            .unwrap();
        fleet.carriers[0].published_state = FlightState::Hover;
        fleet.carriers[0].damage.wire.conditions = 4;
        assert!(candidate(&engine, &source).unwrap().after.is_none());
        let fleet = engine
            .domination_gameplay
            .as_mut()
            .unwrap()
            .state
            .helo
            .as_mut()
            .unwrap();
        fleet.carriers[0].damage.state = VehicleState::Destroyed;
        fleet.carriers[0].damage.wire.conditions = 0;
        assert!(candidate(&engine, &source).unwrap().after.is_none());
    }

    #[test]
    fn rocket_helo_uses_common_friendly_scaling_and_flight_floors() {
        let (mut engine, mut source) = fixture();
        for (flight, health) in [(FlightState::Inbound, 360), (FlightState::Outbound, 320)] {
            engine
                .domination_gameplay
                .as_mut()
                .unwrap()
                .state
                .helo
                .as_mut()
                .unwrap()
                .carriers[0]
                .published_state = flight;
            let after = candidate(&engine, &source).unwrap().after.unwrap();
            assert_eq!(after.carriers[0].damage.wire.health_quarter_points, health);
        }

        let spawns = engine.spawn_catalog.as_ref().unwrap();
        engine.active.as_mut().unwrap().squad_catalog =
            std::sync::Arc::new(spawns.for_squad(0).unwrap());
        source.team = 0;
        let friendly = candidate(&engine, &source).unwrap();
        let after = friendly.after.unwrap();
        // (60 × 0.25 - 6) × 0.5 = 4.5 HP friendly damage.
        assert_eq!(after.carriers[0].damage.wire.health_quarter_points, 382);
        assert_eq!(friendly.audit[0]["friendly"], true);
    }
}
