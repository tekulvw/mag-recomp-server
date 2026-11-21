//! Ground effects from the vehicle channel of an authoritative hospital rocket.
use super::*;
use crate::{
    hospital_rocket_ground::Catalog, hospital_rocket_runtime::LaunchSource, hospital_rockets::Blast,
};

#[derive(Clone, Copy)]
pub(crate) struct GroundRocketDose {
    vehicle_hp: f32,
    half_radius_m: f32,
    outer_radius_m: f32,
    material_penetration: f32,
}

impl From<Blast> for GroundRocketDose {
    fn from(blast: Blast) -> Self {
        Self {
            vehicle_hp: blast.vehicle_hp,
            half_radius_m: blast.half_radius_m,
            outer_radius_m: blast.outer_radius_m,
            material_penetration: blast.material_penetration,
        }
    }
}

pub(crate) struct GroundRocketCandidate {
    pub(crate) after: Option<ground::Runtime>,
    pub(crate) packet: Option<Vec<u8>>,
    pub(crate) audit: Vec<Value>,
}

impl<W: Write> Engine<W> {
    /// The caller owns the shared terminal record and only installs a successful
    /// candidate after durable acceptance.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_hospital_rocket_ground_blast(
        &self,
        source: &LaunchSource,
        origin: [f64; 3],
        dose: GroundRocketDose,
        rules: &Catalog,
        bindings: &ContactBindings,
        collision: BlastCollision<'_>,
        now: Duration,
    ) -> Result<GroundRocketCandidate, &'static str> {
        let attacker = self.hospital_rocket_blast_attacker(source)?;
        let origin_f32 = origin.map(|component| component as f32);
        if !origin_f32.into_iter().all(f32::is_finite)
            || !dose.vehicle_hp.is_finite()
            || dose.vehicle_hp <= 0.0
            || !dose.half_radius_m.is_finite()
            || !dose.outer_radius_m.is_finite()
            || dose.half_radius_m <= 0.0
            || dose.outer_radius_m <= dose.half_radius_m
            || !dose.material_penetration.is_finite()
            || dose.material_penetration < 0.0
        {
            return Err("invalid_hospital_rocket_ground_blast");
        }
        let runtime = self
            .ground_runtime
            .as_ref()
            .ok_or("hospital_rocket_ground_runtime_required")?;
        let policy = self
            .ground_damage_policy()
            .ok_or("hospital_rocket_ground_policy_required")?;
        let snapshot = self.dynamic_collision_snapshot(collision.filter_rows[6])?;
        if snapshot.context().0.world_id != source.world {
            return Err("stale_hospital_rocket_ground_world");
        }
        let scene = self
            .collision_world
            .as_ref()
            .filter(|scene| {
                scene.stats().triangles != 0 && scene.metadata().map_name == self.map_name()
            })
            .ok_or("hospital_rocket_ground_static_collision_required")?;

        let explosion = Geometry::Sphere {
            radius: f64::from(dose.outer_radius_m),
        };
        let mut overlap_budget = collision.limits.max_support_points;
        let mut static_budget = 1_000_000;
        let mut seen = BTreeSet::new();
        let mut changes = BTreeMap::new();
        let mut candidate = GroundRocketCandidate {
            after: None,
            packet: None,
            audit: Vec::new(),
        };
        for body in snapshot.bodies() {
            let key = body.target.key;
            if body.target.kind != TargetKind::GroundVehicle
                || collision.filter_rows[6] & (1 << body.layer) == 0
                || bindings.ground_phantom(&body.root)
                || seen.contains(&(key.id, key.generation))
            {
                continue;
            }
            let index = key.id as u16;
            let Some(vehicle) = runtime.get(index).filter(|vehicle| {
                key.id == u64::from(vehicle.key.object_id())
                    && vehicle.key.generation == key.generation
                    && matches!(vehicle.source_spawn_type, Some(3 | 7))
                    && vehicle.state == 2
                    && vehicle.body == ground::Body::Operational
                    && !vehicle.is_dead()
                    && !vehicle.is_dying()
            }) else {
                continue;
            };
            if !geometry::penetrates(
                &explosion,
                Pose::at(origin),
                &body.shape,
                body.pose,
                collision.limits,
                &mut overlap_budget,
            )? {
                continue;
            }
            seen.insert((key.id, key.generation));

            let creation = self
                .current_ground_vehicle(index)
                .ok_or("unknown_ground_vehicle")?;
            // The admitted root may be offset from the current model origin.
            let pose = crate::dynamic_collision_runtime::vehicle_pose(&creation);
            let anchor = [
                creation.position.x,
                creation.position.y,
                creation.position.z,
            ];
            let (displacement, distance_squared) = separation(origin_f32, anchor)?;
            let scale = armor::native_radial_scale(
                distance_squared.sqrt(),
                dose.half_radius_m,
                dose.outer_radius_m,
            )?;
            if scale <= 0.0
                || !collision.clear_path(
                    scene,
                    &snapshot,
                    BlastRay {
                        start: origin,
                        end: pose.translation,
                        excludes: [Some(key.id), None],
                        penetration: dose.material_penetration,
                        zero_penetration_strike: false,
                        filter_static: true,
                    },
                    &mut static_budget,
                )?
            {
                continue;
            }
            let location = rules.location(&creation, pose, origin)?;
            let friendly = source.team == creation.team;
            let amount = policy.hospital_rocket_amount(
                &creation,
                dose.vehicle_hp,
                scale,
                location,
                friendly,
            )?;
            if amount <= 0.0 {
                continue;
            }
            let user_data = character::explosion_direction_user_data(direction(
                displacement,
                distance_squared,
            ))?;
            let after = candidate.after.get_or_insert_with(|| runtime.clone());
            if after
                .apply_hospital_rocket_damage(vehicle.key, amount, attacker, location, user_data)?
                .is_empty()
            {
                continue;
            }
            changes.insert(index, 1 << 18);
            candidate.audit.push(json!({
                "vehicle_index": index, "generation": key.generation, "location": location,
                "projectile_id": source.projectile_id, "attacker": attacker, "friendly": friendly,
                "radial_scale": scale, "amount_hp": amount,
                "health_before_quarter_points": vehicle.health_quarter_points,
                "health_after_quarter_points": after.get(index).unwrap().health_quarter_points,
            }));
        }
        if changes.is_empty() {
            candidate.after = None;
        } else if let Some(after) = &candidate.after {
            candidate.packet = self.encode_ground_changes(after, &changes, now)?;
        }
        Ok(candidate)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Arc;

    fn ground_rocket_fixture() -> (Engine<Vec<u8>>, LaunchSource, Catalog) {
        let mut engine = crate::hospital_rocket_runtime::tests::tick_fixture();
        let mut ground_fixture = crate::manual_ground_repair_domination_tests::fixture(0, 1);
        engine.ground_vehicles = ground_fixture.ground_vehicles.take();
        engine.ground_runtime = ground_fixture.ground_runtime.take();
        // This fixture introduces already-published Ground objects into the
        // pre-ready rocket fixture; preserve the normal creation-before-update gate.
        let batches = engine.ground_vehicles.as_ref().unwrap().batch_count();
        for actor in engine.active.iter_mut().chain(engine.peers.values_mut()) {
            actor.ground_vehicle_batch = batches;
        }
        let runtime = engine.ground_runtime.as_mut().unwrap();
        let key = runtime.get(0).unwrap().key;
        let q = std::f32::consts::FRAC_1_SQRT_2;
        runtime
            .force_pose(
                key,
                ground::Pose {
                    position: replication::CharacterPosition {
                        x: 1.25,
                        y: 2.5,
                        z: 4.5,
                    },
                    rotation: [0., q, 0., q],
                },
            )
            .unwrap();
        let state = &mut engine.domination_gameplay.as_mut().unwrap().state;
        state.filter_rows[6] |= 1 << 22;
        state.filter_rows[24] = (1 << 9) | 1;
        state.ground_damage = Arc::new(
            serde_json::from_value(json!({
                "friendly_fire_multiplier":0.5,"profiles":[{
                    "vehicle_type_token":110,"faction":0,"max_health":150.,
                    "critical_loss_percent_per_second":0.,
                    "armor":{"rating":10.,"prevention":0.5,"coverage":[0.5,0.5,0.5,0.5]}
                }],
                "areas":vec![json!({"armor_area":0,"multiplier":1.});16]
            }))
            .unwrap(),
        );
        let digest = "a".repeat(64);
        let mut roots = vec![json!({
            "id":"chassis","layer":22,"translation":[0.,0.,2.],"rotation":"identity",
            "shape":{"kind":"sphere","radius":0.3}
        })];
        let mut secondary = roots[0].clone();
        secondary["id"] = json!("second-chassis-part");
        roots.push(secondary);
        roots.push(json!({
            "id":"wheel","layer":22,"translation":[0.,0.,-3.],"rotation":"entity",
            "shape":{"kind":"sphere","radius":0.3}
        }));
        let bodies = dynamic_collision::Catalog::from_json(&json!({
            "format":"mag_dynamic_bodies","schema_version":1,
            "source":{"native_binding_sha256":digest,"shape_data_sha256":digest},
            "templates":[
                {"id":"synthetic-character","kind":"character","roots":[
                    {"id":"capsule","layer":21,"dead_layer":6,"translation":[0.,0.,0.],"rotation":"identity",
                     "shape":{"kind":"capsule","a":[0.,0.,0.],"b":[0.,2.,0.],"radius":0.25}}
                ]},
                {"id":"synthetic-vehicle","kind":"ground_vehicle","roots":roots}
            ]
        }).to_string()).unwrap();
        let bindings = crate::hospital_rocket_runtime::tests::test_contact_bindings(
            &engine,
            engine.collision_world.as_ref().unwrap().source_sha256(),
            bodies.fingerprint(),
        );
        engine.projectile_contacts = Some(Arc::clone(&bindings));
        engine
            .domination_gameplay
            .as_mut()
            .unwrap()
            .state
            .projectile_contacts = Some(bindings);
        let active = engine.active.take();
        let peers = std::mem::take(&mut engine.peers);
        engine = engine
            .with_dynamic_collision_bodies(
                bodies,
                crate::dynamic_collision_runtime::RuntimeBindings {
                    character_template: "synthetic-character".into(),
                    vehicles: vec![
                        crate::dynamic_collision_runtime::VehicleBinding {
                            vehicle_type_token: 110,
                            faction: 0,
                            template: "synthetic-vehicle".into(),
                        },
                        crate::dynamic_collision_runtime::VehicleBinding {
                            vehicle_type_token: 111,
                            faction: 1,
                            template: "synthetic-vehicle".into(),
                        },
                    ],
                },
            )
            .unwrap();
        engine.active = active;
        engine.peers = peers;
        let actor = engine.active.as_ref().unwrap();
        let source = LaunchSource {
            reservation: actor.reservation.id.clone(),
            world: actor.reservation.world,
            round: engine.round_number,
            match_id: actor.reward_match_id.clone(),
            character_id: actor.reservation.character.character_id,
            player: actor.reservation.player,
            life_generation: 1,
            team: actor.team(),
            item: 42,
            projectile_id: 0x1380_0000,
        };
        (engine, source, Catalog::synthetic(110, 111))
    }

    fn plan(
        engine: &Engine<Vec<u8>>,
        source: &LaunchSource,
        rules: &Catalog,
        origin: [f64; 3],
    ) -> Result<GroundRocketCandidate, &'static str> {
        let state = &engine.domination_gameplay.as_ref().unwrap().state;
        engine.prepare_hospital_rocket_ground_blast(
            source,
            origin,
            GroundRocketDose {
                vehicle_hp: 250.0,
                half_radius_m: 2.0,
                outer_radius_m: 4.0,
                material_penetration: 0.0,
            },
            rules,
            engine.projectile_contacts.as_deref().unwrap(),
            BlastCollision {
                filter_rows: &state.filter_rows,
                limits: QueryLimits::default(),
                sequences: &state.collisions,
                material_costs: &BTreeMap::from([(1, 100.0)]),
                terrain_materials: None,
            },
            Duration::from_millis(1000),
        )
    }

    #[test]
    fn rocket_ground_plans_once_from_current_model_pose_without_changing_live_authority() {
        let (engine, source, rules) = ground_rocket_fixture();
        let plan = plan(&engine, &source, &rules, [1.5, 2.5, 4.0]).unwrap();
        let after = plan.after.unwrap();
        let damage = after.get(0).unwrap().damage_wire();
        assert_eq!(damage.last_location_and_type, 0xa2);
        assert_eq!(damage.last_attacker, 0x0380_0000);
        assert_eq!(damage.last_source_object, u32::MAX);
        assert!(after.get(0).unwrap().pending_death);
        assert_eq!(plan.audit.len(), 1);
        assert_eq!(plan.audit[0]["projectile_id"], source.projectile_id);
        let packet = plan.packet.unwrap();
        let decoded =
            replication::ObjectUpdate::decode_with_profile(&packet, engine.wire_profile).unwrap();
        assert_eq!(decoded.header.object_type_mask, 1 << 1);
        let mut bits = replication::BitReader::with_profile(&packet, engine.wire_profile);
        bits.position = engine.wire_profile.header_bits();
        assert_eq!(bits.read(1), Ok(1));
        assert_eq!(bits.read(16), Ok(0));
        assert_eq!(bits.read(18), Ok(0));
        assert_eq!(bits.read(1), Ok(1));
        assert_eq!(bits.read(8), Ok(0xa2));
        assert_eq!(bits.read(16).unwrap() & 0xfff, 0);
        assert_eq!(bits.read(32), Ok(0x0380_0000));
        assert_eq!(bits.read(32), Ok(u32::MAX));
        assert_eq!(bits.read(32), Ok(damage.last_user_data));
        assert_eq!(bits.read(12), Ok(0));
        assert_eq!(
            engine
                .ground_runtime
                .as_ref()
                .unwrap()
                .get(0)
                .unwrap()
                .health_quarter_points,
            600
        );
        assert!(
            !engine
                .ground_runtime
                .as_ref()
                .unwrap()
                .get(0)
                .unwrap()
                .pending_death
        );
    }

    #[test]
    fn rocket_ground_skips_cover_and_phantom_only_overlap_and_rejects_stale_credit() {
        let (engine, mut source, rules) = ground_rocket_fixture();
        for origin in [[1.25, 2.5, 2.5], [-1.75, 2.5, 4.5]] {
            let planned = plan(&engine, &source, &rules, origin).unwrap();
            assert!(
                planned.after.is_none() && planned.packet.is_none(),
                "{origin:?}: {:?}",
                planned.audit
            );
        }
        source.round += 1;
        assert!(matches!(
            plan(&engine, &source, &rules, [1.5, 2.5, 4.0]),
            Err("stale_hospital_rocket_character_source")
        ));
    }

    const TARGET: &str = "synthetic-rocket-target";
    const CONTACT_AT: Duration = Duration::from_millis(1050);

    pub(crate) fn combined_fixture(enable_ground: bool) -> Engine<Vec<u8>> {
        let (mut engine, _, rules) = ground_rocket_fixture();
        if enable_ground {
            engine.hospital_rocket_ground = Some(Arc::new(rules));
        }
        let ground = engine.ground_runtime.as_mut().unwrap();
        let vehicle = ground.get(0).unwrap();
        let mut pose = vehicle.pose;
        // The rocket hits the existing test wall. Both targets are on its near
        // side, apart from the connected rocket's direct contact corridor.
        pose.position.z = 2.0;
        ground.force_pose(vehicle.key, pose).unwrap();
        crate::hospital_rocket_runtime::tests::launch_for_tick(&mut engine);
        engine
    }

    fn terminal_audits(engine: &Engine<Vec<u8>>) -> Vec<Value> {
        engine
            .writer
            .split(|&byte| byte == b'\n')
            .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
            .filter(|row| row["op"] == "hospital_rocket_terminal")
            .collect()
    }

    fn live_damage(
        engine: &Engine<Vec<u8>>,
    ) -> (replication::CharacterDamage, replication::VehicleDamage) {
        (
            engine.peers[TARGET].last_damage,
            engine
                .ground_runtime
                .as_ref()
                .unwrap()
                .get(0)
                .unwrap()
                .damage_wire(),
        )
    }

    fn flight_snapshot(engine: &Engine<Vec<u8>>) -> Vec<Vec<u8>> {
        engine
            .active
            .as_ref()
            .unwrap()
            .hospital_rocket_flights
            .snapshot_packets(1050, engine.wire_profile)
            .unwrap()
    }

    #[test]
    fn rocket_ground_and_character_share_one_terminal_record_and_publication() {
        let mut engine = combined_fixture(true);
        let old = live_damage(&engine);
        engine.tick_hospital_rockets(CONTACT_AT, 1).unwrap();
        let (character, ground) = live_damage(&engine);
        let audit = terminal_audits(&engine);
        assert_eq!(audit.len(), 1);
        assert!(audit[0]["damage_denial"].is_null(), "{audit:?}");
        assert!(character.health_quarter_points < old.0.health_quarter_points);
        assert_eq!(character.last_source_object, 0x1380_0000);
        assert!(
            ground.health_quarter_points < old.1.health_quarter_points,
            "{audit:?}"
        );
        assert_eq!(ground.last_source_object, u32::MAX);
        // The same terminal reached both effects, followed by one cleanup state.
        assert!(flight_snapshot(&engine).is_empty());
        let masks: Vec<_> = engine.peers[TARGET]
            .pending_shared
            .iter()
            .map(|packet| {
                replication::ObjectUpdate::decode_with_profile(packet, engine.wire_profile)
                    .unwrap()
                    .header
                    .object_type_mask
            })
            .collect();
        assert!(
            masks.windows(2).any(|masks| masks == [1 << 1, 1 << 19]),
            "{masks:?}"
        );
    }

    #[test]
    fn rocket_ground_and_character_roll_back_together_on_terminal_record_refusal() {
        let mut engine = combined_fixture(true);
        let before = live_damage(&engine);
        let flying = flight_snapshot(&engine);
        let shooter_queue = engine.active.as_ref().unwrap().pending_shared.clone();
        let target_queue = engine.peers[TARGET].pending_shared.clone();
        engine.max_log = engine.written;
        assert_eq!(
            engine.tick_hospital_rockets(CONTACT_AT, 1).err(),
            Some("log_limit_reached")
        );
        assert_eq!(format!("{:?}", live_damage(&engine)), format!("{before:?}"));
        assert_eq!(flight_snapshot(&engine), flying);
        assert_eq!(
            engine.active.as_ref().unwrap().pending_shared,
            shooter_queue
        );
        assert_eq!(engine.peers[TARGET].pending_shared, target_queue);
        assert!(terminal_audits(&engine).is_empty());
    }

    fn fill_for_two_packets(engine: &mut Engine<Vec<u8>>) {
        let ordinary = replication::TeamSpawnDeadline {
            sequence: 1,
            timestamp_ms: 1020,
            object_index: 0,
            team: 0,
            deadline_ms: 3000,
        }
        .encode_with_profile(engine.wire_profile)
        .unwrap();
        let actor = engine.active.as_mut().unwrap();
        while actor.ordinary_shared_len() < multiplayer::MAX_SHARED_QUEUE - 2 {
            actor.queue_shared(ordinary.clone());
        }
    }

    #[test]
    fn rocket_ground_requires_one_more_fifo_slot_and_retries_after_normal_drain() {
        let mut character_only = combined_fixture(false);
        fill_for_two_packets(&mut character_only);
        character_only.tick_hospital_rockets(CONTACT_AT, 1).unwrap();
        assert_eq!(terminal_audits(&character_only).len(), 1);

        let mut combined = combined_fixture(true);
        fill_for_two_packets(&mut combined);
        let before = live_damage(&combined);
        let flying = flight_snapshot(&combined);
        assert!(
            combined
                .tick_hospital_rockets(CONTACT_AT, 1)
                .unwrap()
                .is_empty()
        );
        assert!(terminal_audits(&combined).is_empty());
        assert_eq!(
            format!("{:?}", live_damage(&combined)),
            format!("{before:?}")
        );
        assert_eq!(flight_snapshot(&combined), flying);
        assert!(!combined.drain_shared(Vec::new(), 1).unwrap().is_empty());
        combined
            .tick_hospital_rockets(Duration::from_millis(1100), 1)
            .unwrap();
        assert_eq!(terminal_audits(&combined).len(), 1);
        let (character, ground) = live_damage(&combined);
        assert!(character.health_quarter_points < before.0.health_quarter_points);
        assert!(ground.health_quarter_points < before.1.health_quarter_points);
    }
}
