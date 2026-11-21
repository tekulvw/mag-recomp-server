//! Record-bound ordinary retail frag damage from the owner event's accepted pose.
use crate::{
    Active, CharacterLife, Engine, PendingDamage, acquisition_character_effects, armor,
    blast_visibility::MAX_BLAST_TARGETS,
    damage,
    domination_structural_world::{BlastCollision, CharacterBlastShape, OnFootBlastPose},
    dynamic_collision_runtime::OwnerCollisionView,
    leadership_runtime::LeadershipPlan,
    projectile::{ProjectileTerminal, ProjectileTerminalReason},
};
use serde_json::{Value, json};
use std::io::Write;

pub(crate) struct FragFrame<'a> {
    pub(crate) pose: OnFootBlastPose,
    pub(crate) ground: Option<&'a crate::acquisition_ground_runtime::Runtime>,
    pub(crate) leadership: &'a LeadershipPlan,
}

#[derive(Default)]
pub(crate) struct FragDamagePlan {
    pub(crate) doses: Vec<PendingDamage>,
    pub(crate) audit: Vec<Value>,
}

impl<W: Write> Engine<W> {
    pub(crate) fn prepare_frag_character_damage(
        &self,
        source: &Active,
        terminals: &[ProjectileTerminal],
        frame: FragFrame<'_>,
    ) -> Result<FragDamagePlan, &'static str> {
        let terminals = terminals
            .iter()
            .filter(|t| t.reason == ProjectileTerminalReason::ReportedDetonation)
            .filter_map(|t| t.position.map(|p| (t, [p.x, p.y, p.z])));
        let mut terminals = terminals.peekable();
        if terminals.peek().is_none() {
            return Ok(FragDamagePlan::default());
        }
        let state = &self
            .domination_gameplay
            .as_ref()
            .ok_or("frag_character_blast_requires_domination")?
            .state;
        let snapshot = self.owner_collision_snapshot(
            state.filter_rows[6],
            OwnerCollisionView {
                active_position: frame.pose.position,
                ground: frame.ground,
            },
        )?;
        let collision = BlastCollision {
            filter_rows: &state.filter_rows,
            limits: Default::default(),
            sequences: &state.collisions,
            material_costs: state.structural.material_penetration_costs(),
            terrain_materials: Some(&state.terrain_materials),
        };

        let catalog = self.item_catalog.retail_combat()?;
        let rules = self.item_catalog.character_blast_rules();
        let mut plan = FragDamagePlan::default();
        let player = u32::from(source.reservation.player);
        let attacker = 0x0380_0000 | player;
        let world = source.reservation.world;
        for (terminal, origin) in terminals {
            let frag = catalog
                .grenade_for_damage(terminal.item_id)
                .map_err(|_| "frag_character_blast_item_unavailable")?;
            let shape = CharacterBlastShape {
                origin: origin.map(f64::from),
                half_radius_m: frag.half_radius as f32,
                outer_radius_m: frag.full_radius as f32,
                material_penetration: frag
                    .material_penetration
                    .ok_or("frag_character_blast_material_required")?
                    as f32,
            };
            let observations =
                self.collect_character_blast(world, shape, collision, &snapshot, frame.pose)?;
            if observations.len() > MAX_BLAST_TARGETS {
                return Err("invalid_blast_roster");
            }
            let source_object_id = ((16 + u32::from(terminal.slot)) << 24) | 0x0080_0000 | player;
            for sample in observations {
                let target = sample.target;
                let actor = self.active.iter().chain(self.peers.values()).find(|actor| {
                    actor.reservation.world == target.world
                        && (0x0380_0000 | u32::from(actor.reservation.player))
                            == target.character_id
                });
                let (reservation, armor) = if let Some(actor) = actor {
                    if !actor.ready_for_updates
                        || !actor.entered_game
                        || actor.initial_objects.is_none()
                    {
                        continue;
                    }
                    let kit = actor.equipped_loadout().ok_or("missing_target_loadout")?;
                    let armor = armor::ArmorProfile::from_retail_loadout(&kit, catalog)?
                        .with_modifiers(&frame.leadership.modifiers(actor));
                    let armor = if matches!(actor.life, CharacterLife::Incapacitated { .. }) {
                        armor.for_incapacitated(catalog)?
                    } else {
                        armor
                    };
                    (Some(actor.reservation.id.clone()), armor)
                } else {
                    let kit = self
                        .registry_bot_loadout(target.character_id as u16)
                        .ok_or("missing_target_loadout")?;
                    (
                        None,
                        armor::ArmorProfile::from_retail_loadout(kit, catalog)?,
                    )
                };
                let raw = armor::native_character_blast_damage(
                    frag.damage.character as f32 * sample.scale,
                    rules,
                );
                let armor = armor.with_incoming_base(self.character_incoming_base(
                    attacker,
                    target.character_id,
                    world,
                ))?;
                let kind = damage::DamageKind::Explosive;
                let amount_hp = armor.adjust_for_kind(raw, kind)?;
                if amount_hp == 0. {
                    continue;
                }
                let user_data =
                    acquisition_character_effects::explosion_direction_user_data(sample.direction)?;
                plan.doses.push(PendingDamage {
                    target_character_id: target.character_id,
                    target_reservation_id: reservation,
                    id: format!(
                        "grenade-{}-{}-{:08x}",
                        terminal.slot, terminal.generation, target.character_id
                    ),
                    generation: target.generation,
                    damage: damage::ResolvedDamage {
                        amount_hp,
                        kind,
                        kinetic_response: rules.damage_type & 8 != 0,
                        location: raw.hit_location,
                        attacker_id: attacker,
                        source_object_id,
                        user_data,
                    },
                    falling: false,
                });
                plan.audit.push(json!({
                    "slot": terminal.slot, "generation": terminal.generation,
                    "character_id": target.character_id, "life_generation": target.generation,
                    "origin": origin, "distance": sample.distance, "amount_hp": amount_hp,
                    "hit_location": raw.hit_location, "damage_type": rules.damage_type,
                    "user_data": user_data, "source_object": source_object_id,
                    "damage_calculation": armor.calculation(raw, kind)?,
                }));
            }
        }
        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        blast_visibility::MAX_BLAST_TARGETS,
        hospital_rocket_runtime::tests as rocket_fixture,
        projectile::{ProjectileOwnerFields, ProjectileOwnerUpdate},
        replication::CharacterPosition,
        spawn_progression::dispatch,
        *,
    };

    const ORIGIN: CharacterPosition = CharacterPosition {
        x: 1.,
        y: 2.5,
        z: 1.,
    };

    fn fixture() -> Engine<Vec<u8>> {
        let mut engine = rocket_fixture::tick_fixture();
        engine.frag_character_blast = true;
        engine.development_grenade_radius = false;
        engine.item_catalog = human_kill_reward_tests::grenade_items_with_cover();
        let mut human = human_kill_reward_tests::fixture();
        let equipped = human.active.as_mut().unwrap();
        let mut kit = equipped.spawned_loadout.take().unwrap();
        kit.inventories[0].slots[4] = Some(4);
        let source = engine.active.as_mut().unwrap();
        source.mounted_bunker = None;
        source.mounted_seat = None;
        source.grenade_remaining = 1;
        source.current_inventory_handle = equipped.current_inventory_handle;
        source.spawned_loadout = Some(kit);
        source.life = CharacterLife::Alive {
            generation: 4,
            health_quarter_points: 400,
        };
        source.owner_fields.state = Some(replication::CharacterState {
            state: 1,
            stance: 2,
            user_data: [0; 3],
        });
        source.tracking.server_position(
            CharacterPosition { x: 20., ..ORIGIN },
            player_tracking::PositionOrigin::ServerSpawn,
            1,
        );
        let target = engine.peers.get_mut("synthetic-rocket-target").unwrap();
        target.owner_fields.state = Some(replication::CharacterState {
            state: 1,
            stance: 0,
            user_data: [0; 3],
        });
        target.tracking.server_position(
            CharacterPosition {
                x: 2.,
                y: 0.5,
                z: 1.,
            },
            player_tracking::PositionOrigin::ServerSpawn,
            1,
        );
        let wire = rocket_fixture::launch_wire(&engine, 3, 4);
        assert!(!dispatch(&mut engine, &wire, 1000).unwrap().is_empty());
        assert!(engine.active.as_ref().unwrap().projectiles.get(3).is_some());
        engine
    }

    fn update(engine: &Engine<Vec<u8>>, stamp: u32, pose: bool, terminal: bool) -> Vec<u8> {
        report(
            engine,
            stamp,
            pose,
            terminal.then(|| engine.active.as_ref().unwrap().reservation.player),
            None,
        )
    }

    fn report(
        engine: &Engine<Vec<u8>>,
        stamp: u32,
        pose: bool,
        terminal: Option<u16>,
        ground: Option<CharacterPosition>,
    ) -> Vec<u8> {
        let actor = engine.active.as_ref().unwrap();
        let mut bits = replication::BitWriter::with_profile(engine.wire_profile);
        replication::UpdateHeader {
            sequence: stamp,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: (u32::from(ground.is_some()) << 1)
                | (u32::from(pose) << 3)
                | (u32::from(terminal.is_some()) << 19),
            timestamp_ms: stamp,
        }
        .encode(&mut bits)
        .unwrap();
        if let Some(position) = ground {
            bits.write(1, 1).unwrap();
            bits.write(0, 16).unwrap(); // Ground vehicle index
            bits.write(0, 25).unwrap();
            bits.write(1, 1).unwrap(); // Field 25 position
            position.encode(&mut bits).unwrap();
            bits.write(0, 5).unwrap(); // Fields 26–29 and list terminator
        }
        if pose {
            bits.write(1, 1).unwrap();
            bits.write(actor.reservation.player.into(), 16).unwrap();
            for field in 0..22 {
                bits.write(u32::from(matches!(field, 4 | 5)), 1).unwrap();
                match field {
                    4 => replication::CharacterState {
                        state: 1,
                        stance: 0,
                        user_data: [0; 3],
                    }
                    .encode(&mut bits)
                    .unwrap(),
                    5 => CharacterPosition { y: 0.5, ..ORIGIN }
                        .encode(&mut bits)
                        .unwrap(),
                    _ => (),
                }
            }
            bits.write(0, 1).unwrap();
        }
        if let Some(player) = terminal {
            bits.write(1, 1).unwrap();
            bits.write(player.into(), 16).unwrap();
            ProjectileOwnerUpdate {
                slot: 3,
                object_index: player,
                fields: ProjectileOwnerFields {
                    state: Some(2),
                    position: Some(ORIGIN),
                    rotation_xyzw: None,
                },
            }
            .fields
            .encode_fields(&mut bits)
            .unwrap();
            bits.write(0, 1).unwrap();
        }
        bits.finish()
    }

    fn health(actor: &Active) -> u16 {
        actor.life.damage_health().unwrap().1
    }

    fn last_event(engine: &Engine<Vec<u8>>) -> Value {
        String::from_utf8_lossy(&engine.writer)
            .lines()
            .rev()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find(|row| row["op"] == "event")
            .unwrap()
    }

    fn update_on_tcp(engine: &mut Engine<Vec<u8>>, wire: &[u8], now_ms: u64) {
        let actor = engine.active.as_ref().unwrap();
        let input = events::EventInput {
            reservation_id: actor.reservation.id.clone(),
            world_id: actor.reservation.world,
            player_id: actor.reservation.player,
            transport: Transport::Tcp,
            direction: Direction::ClientToServer,
            rt_type: 0x3d,
            plugin_id: Some(1),
            payload_hex: protocol::encode_hex(wire),
        };
        let decoded = DecodedEvent::with_profile(wire, input.player_id, engine.wire_profile, false);
        engine
            .handle_event(input, &decoded, Duration::from_millis(now_ms), 1)
            .unwrap();
    }

    fn wall(engine: &Engine<Vec<u8>>) -> collision::CollisionWorld {
        collision::CollisionWorld::from_bytes(
            json!({
                "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
                "metadata":{"map_name":engine.map_name(),"build":"synthetic",
                    "coverage":"provisional","provenance":["synthetic"],"unknowns":[]},
                "instances":[{"id":"frag-wall","vertices_xyz":[[1.5,1.,0.],[1.5,1.,2.],[1.5,5.,1.]],
                    "triangles":[[0,1,2]],"material_index":255,
                    "collision":{"filter_info":9,"ray_child_filter_info":0,"convex_radius":0.,
                        "provenance":"synthetic","child_filter_policy":"uniform"}}]
            })
            .to_string()
            .as_bytes(),
            collision::CollisionLimits::default(),
        )
        .unwrap()
    }

    #[test]
    fn retail_frag_uses_fuse_and_same_event_accepted_self_pose_and_stance() {
        let mut engine = fixture();
        let early = update(&engine, 1099, false, true);
        dispatch(&mut engine, &early, 1099).unwrap();
        assert!(
            !engine
                .active
                .as_ref()
                .unwrap()
                .projectiles
                .get(3)
                .unwrap()
                .completed
        );
        assert_eq!(health(&engine.peers["synthetic-rocket-target"]), 400);

        let terminal = update(&engine, 1100, true, false);
        dispatch(&mut engine, &terminal, 1100).unwrap();
        let actor = engine.active.as_ref().unwrap();
        assert!(actor.projectiles.get(3).unwrap().completed);
        // Self moved from 20m to the blast center. Stance 0 places its anchor
        // at the center, so the synthetic 20 HP dose has no radial attenuation.
        assert_eq!(health(actor), 320);
        let queued = actor.grenade_damage.front().unwrap();
        assert_eq!(queued.damage.amount_hp, 17.5); // Linear 20 HP at 1m of half-radius 4m.
        assert_eq!(queued.damage.user_data, 128 << 20); // Positive X, exponent 7.
        assert_eq!(queued.damage.source_object_id, 0x1380_0000);
        assert_eq!(queued.damage.location, 3);
        assert!(queued.damage.kinetic_response);

        let drain = update(&engine, 1101, false, false);
        dispatch(&mut engine, &drain, 1101).unwrap();
        assert_eq!(health(&engine.peers["synthetic-rocket-target"]), 330);
        assert!(engine.active.as_ref().unwrap().grenade_damage.is_empty());
    }

    #[test]
    fn retail_frag_rejects_foreign_tcp_and_stale_owner_terminal_claims() {
        let mut engine = fixture();
        let foreign = report(&engine, 1100, false, Some(1), None);
        assert_eq!(
            dispatch(&mut engine, &foreign, 1100).unwrap_err(),
            "unowned_projectile_update"
        );
        let tcp = update(&engine, 1101, true, true);
        update_on_tcp(&mut engine, &tcp, 1101);
        let actor = engine.active.as_ref().unwrap();
        assert_eq!(actor.owner_fields.state.unwrap().stance, 2);
        assert!(!actor.projectiles.get(3).unwrap().completed);

        let new_sequence = update(&engine, 1110, false, false);
        dispatch(&mut engine, &new_sequence, 1102).unwrap();
        let stale = update(&engine, 1109, false, true);
        dispatch(&mut engine, &stale, 1103).unwrap();
        let actor = engine.active.as_ref().unwrap();
        assert!(!actor.projectiles.get(3).unwrap().completed);
        assert!(actor.grenade_damage.is_empty());
        assert_eq!(health(&engine.peers["synthetic-rocket-target"]), 400);
    }

    #[test]
    fn retail_frag_blocks_cover_and_keeps_terminal_when_cover_unavailable() {
        for unavailable in [false, true] {
            let mut engine = fixture();
            engine.collision_world = (!unavailable).then(|| wall(&engine));
            let terminal = update(&engine, 1100, false, true);
            dispatch(&mut engine, &terminal, 1100).unwrap();
            let actor = engine.active.as_ref().unwrap();
            assert!(actor.projectiles.get(3).unwrap().completed);
            assert!(engine.projectile_cleanup_pending(actor, 3));
            assert!(actor.grenade_damage.is_empty());
            assert_eq!(health(&engine.peers["synthetic-rocket-target"]), 400);
            let log = last_event(&engine);
            if unavailable {
                assert_eq!(
                    log["decoded"]["native_grenade_character_checks"][0]["error"],
                    "character_blast_static_collision_required"
                );
            } else {
                assert!(
                    log["decoded"]["native_grenade_character_checks"].is_null(),
                    "{log}"
                );
                assert!(log["gameplay_denial"].is_null(), "{log}");
            }
        }
    }

    #[test]
    fn retail_frag_source_damage_does_not_duplicate_development_or_replayed_doses() {
        let mut engine = fixture();
        engine.development_grenade_radius = true;
        let terminal = update(&engine, 1100, false, true);
        dispatch(&mut engine, &terminal, 1100).unwrap();
        assert_eq!(health(&engine.peers["synthetic-rocket-target"]), 330);
        assert!(
            last_event(&engine)["decoded"]["grenade_blast_checks"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        dispatch(&mut engine, &terminal, 1101).unwrap();
        assert_eq!(health(&engine.peers["synthetic-rocket-target"]), 330);
        assert!(engine.active.as_ref().unwrap().grenade_damage.is_empty());
    }

    #[test]
    fn retail_frag_recorder_refusal_preserves_pose_flight_queue_and_health() {
        let mut engine = fixture();
        let terminal = update(&engine, 1100, true, true);
        engine.max_log = engine.written;
        assert_eq!(
            dispatch(&mut engine, &terminal, 1100).unwrap_err(),
            "log_limit_reached"
        );
        let actor = engine.active.as_ref().unwrap();
        assert_eq!(actor.tracking.sample.unwrap().position.x, 20.);
        assert_eq!(actor.owner_fields.state.unwrap().stance, 2);
        assert!(!actor.projectiles.get(3).unwrap().completed);
        assert!(!engine.projectile_cleanup_pending(actor, 3));
        assert!(actor.grenade_damage.is_empty());
        assert_eq!(health(actor), 400);
        assert_eq!(health(&engine.peers["synthetic-rocket-target"]), 400);
    }

    #[test]
    fn retail_frag_queued_dose_cannot_damage_the_victims_next_life() {
        let mut engine = fixture();
        let terminal = update(&engine, 1100, true, true);
        dispatch(&mut engine, &terminal, 1100).unwrap();
        let queue = &engine.active.as_ref().unwrap().grenade_damage;
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].generation, 4);
        engine
            .peers
            .get_mut("synthetic-rocket-target")
            .unwrap()
            .life = CharacterLife::Alive {
            generation: 5,
            health_quarter_points: 400,
        };
        let drain = update(&engine, 1101, false, false);
        dispatch(&mut engine, &drain, 1101).unwrap();
        assert_eq!(health(&engine.peers["synthetic-rocket-target"]), 400);
        assert!(engine.active.as_ref().unwrap().grenade_damage.is_empty());
    }

    #[test]
    fn retail_frag_does_not_expand_the_pending_damage_queue() {
        let mut engine = fixture();
        let queue = &mut engine.active.as_mut().unwrap().grenade_damage;
        let existing = PendingDamage {
            target_character_id: 0x0380_0001,
            target_reservation_id: Some("previous-reservation".to_owned()),
            id: "previous-frag-dose".to_owned(),
            generation: 4,
            damage: damage::ResolvedDamage {
                amount_hp: 20.,
                kind: damage::DamageKind::Explosive,
                kinetic_response: true,
                location: 3,
                attacker_id: 0x0380_0000,
                source_object_id: 0x1280_0000,
                user_data: 0,
            },
            falling: false,
        };
        queue.resize(4 * MAX_BLAST_TARGETS, existing);
        let terminal = update(&engine, 1100, false, true);
        dispatch(&mut engine, &terminal, 1100).unwrap();
        let actor = engine.active.as_ref().unwrap();
        assert!(actor.projectiles.get(3).unwrap().completed);
        assert_eq!(actor.grenade_damage.len(), 4 * MAX_BLAST_TARGETS - 1);
        assert!(
            actor
                .grenade_damage
                .iter()
                .all(|dose| dose.id == "previous-frag-dose")
        );
        assert_eq!(health(&engine.peers["synthetic-rocket-target"]), 400);
    }

    #[test]
    fn retail_frag_sees_staged_ground_pose_before_committing_damage() {
        let mut engine = fixture();
        let mut driver = crate::spawn_progression::tests::ground_vehicle_runtime::domination_ground_driver::driver_engine(0);
        engine.ground_vehicles = driver.ground_vehicles.take();
        engine.ground_runtime = driver.ground_runtime.take();
        let geometry = dynamic_collision::Catalog::from_json(&json!({
            "format":"mag_dynamic_bodies","schema_version":1,
            "source":{"native_binding_sha256":"a".repeat(64),"shape_data_sha256":"b".repeat(64)},
            "templates":[
                {"id":"character","kind":"character","roots":[
                    {"id":"capsule","layer":21,"translation":[0.,0.,0.],"rotation":"identity",
                    "shape":{"kind":"capsule","a":[0.,0.,0.],"b":[0.,2.,0.],"radius":0.25}}]},
                {"id":"vehicle","kind":"ground_vehicle","roots":[
                    {"id":"chassis","layer":22,"translation":[0.,0.,0.],"rotation":"identity",
                    "shape":{"kind":"box","half_extents":[0.125,0.25,0.25],"radius":0.}}]}
            ],
        }).to_string()).unwrap();
        let catalog = engine.ground_vehicles.as_ref().unwrap();
        let vehicles = catalog
            .creations()
            .map(|row| (row.vehicle_type_token, row.faction))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(
                |(vehicle_type_token, faction)| dynamic_collision_runtime::VehicleBinding {
                    vehicle_type_token,
                    faction,
                    template: "vehicle".to_owned(),
                },
            )
            .collect();
        let batches = catalog.batch_count();
        let mut source = engine.active.take().unwrap();
        let mut peers = std::mem::take(&mut engine.peers);
        engine = engine
            .with_dynamic_collision_bodies(
                geometry,
                dynamic_collision_runtime::RuntimeBindings {
                    character_template: "character".to_owned(),
                    vehicles,
                },
            )
            .unwrap();
        source.ground_vehicle_batch = batches;
        source.ground_seat = Some(ground_seats::PassengerSeat {
            vehicle_index: 0,
            seat: 0,
        });
        for peer in peers.values_mut() {
            peer.ground_vehicle_batch = batches;
        }
        engine.active = Some(source);
        engine.peers = peers;
        let previous = CharacterPosition { x: 1.5, ..ORIGIN };
        let ground = engine.ground_runtime.as_mut().unwrap();
        let key = ground.get(0).unwrap().key;
        ground
            .force_pose(
                key,
                acquisition_ground_runtime::Pose {
                    position: previous,
                    rotation: [0., 0., 0., 1.],
                },
            )
            .unwrap();
        ground.begin_driver(0, 1000).unwrap();
        let state = &mut engine.domination_gameplay.as_mut().unwrap().state;
        state.filter_rows[24] = (1 << 9) | (1 << 22);
        let filter_rows = state.filter_rows;
        let old_snapshot = engine.dynamic_collision_snapshot(filter_rows[6]).unwrap();
        assert!(engine.dynamic_collision_snapshot_is_current(&old_snapshot));
        assert!(
            old_snapshot
                .positive_ray_hit_ignoring(
                    [1., 2.5, 1.],
                    [2., 2.5, 1.],
                    [None; 2],
                    filter_rows,
                    Default::default(),
                )
                .unwrap()
        );
        let moved = CharacterPosition { z: 2., ..previous };
        let mut staged = engine.ground_runtime.as_ref().unwrap().clone();
        staged
            .force_pose(
                key,
                acquisition_ground_runtime::Pose {
                    position: moved,
                    rotation: [0., 0., 0., 1.],
                },
            )
            .unwrap();
        let candidate_snapshot = engine
            .owner_collision_snapshot(
                filter_rows[6],
                dynamic_collision_runtime::OwnerCollisionView {
                    active_position: None,
                    ground: Some(&staged),
                },
            )
            .unwrap();
        assert_eq!(candidate_snapshot.context(), old_snapshot.context());
        assert!(!engine.dynamic_collision_snapshot_is_current(&candidate_snapshot));
        let terminal = report(&engine, 1100, false, Some(0), Some(moved));
        dispatch(&mut engine, &terminal, 1100).unwrap();
        assert_eq!(engine.current_ground_vehicle(0).unwrap().position, moved);
        assert_eq!(health(engine.active.as_ref().unwrap()), 400); // Mounted driver.
        assert_eq!(health(&engine.peers["synthetic-rocket-target"]), 330);
    }
}
