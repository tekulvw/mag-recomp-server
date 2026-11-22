//! Classified vehicle blast effects against current operational Helo bodies.
use super::*;
use crate::{
    domination_helo::FleetRuntime,
    domination_helo_damage::{DamageDose, PreparedGrenades, damage_delta},
    dynamic_collision_runtime::OwnerCollisionView,
    hospital_rocket_helo::Catalog,
    hospital_rocket_runtime::LaunchSource,
    hospital_rockets::Blast,
    item_catalog::TargetDamageParameters,
    projectile::{ProjectileTerminal, ProjectileTerminalReason},
};

#[derive(Clone, Copy)]
pub(crate) struct HeloVehicleDose {
    damage: TargetDamageParameters,
    half_radius_m: f32,
    outer_radius_m: f32,
    material_penetration: f32,
}

impl HeloVehicleDose {
    pub(crate) fn equipment(damage: TargetDamageParameters, blast: Blast) -> Self {
        Self {
            damage,
            half_radius_m: blast.half_radius_m,
            outer_radius_m: blast.outer_radius_m,
            material_penetration: blast.material_penetration,
        }
    }
}

impl From<Blast> for HeloVehicleDose {
    fn from(blast: Blast) -> Self {
        Self::equipment(
            TargetDamageParameters {
                amount: blast.vehicle_hp,
                armor_pierce: 0.,
                armor_redux: 0.,
            },
            blast,
        )
    }
}

#[derive(Default)]
pub(crate) struct HeloBlastCandidate {
    pub(crate) after: Option<FleetRuntime>,
    pub(crate) packets: Vec<Vec<u8>>,
    pub(crate) audit: Vec<Value>,
}

struct HeloBlastCredit {
    world: u32,
    attacker: u32,
    team: u8,
}

#[derive(Clone, Copy)]
struct HeloVehicleBlast {
    origin: [f64; 3],
    projectile_id: u32,
    dose: HeloVehicleDose,
}

struct HeloBlastFrame<'a> {
    rules: &'a Catalog,
    collision: BlastCollision<'a>,
    owner_view: Option<OwnerCollisionView<'a>>,
}

impl<W: Write> Engine<W> {
    /// Clone only the qualified current fleet. The caller owns terminal
    /// publication and installs this candidate after its record succeeds.
    pub(crate) fn prepare_hospital_rocket_helo_blast(
        &self,
        source: &LaunchSource,
        origin: [f64; 3],
        dose: HeloVehicleDose,
        rules: &Catalog,
        collision: BlastCollision<'_>,
        now: Duration,
    ) -> Result<HeloBlastCandidate, &'static str> {
        let attacker = self.hospital_rocket_blast_attacker(source)?;
        self.prepare_helo_vehicle_blasts(
            HeloBlastCredit {
                world: source.world,
                attacker,
                team: source.team,
            },
            &[HeloVehicleBlast {
                origin,
                projectile_id: source.projectile_id,
                dose,
            }],
            HeloBlastFrame {
                rules,
                collision,
                owner_view: None,
            },
            now,
        )
    }

    pub(crate) fn prepare_domination_source_helo_frags(
        &self,
        actor: &Active,
        terminals: &[ProjectileTerminal],
        owner_view: OwnerCollisionView<'_>,
        now: Duration,
    ) -> Result<Option<PreparedGrenades>, &'static str> {
        let world = self
            .domination_gameplay
            .as_ref()
            .ok_or("frag_helo_requires_domination_world")?;
        let rules = world
            .state
            .helo_frag_damage_areas
            .as_deref()
            .ok_or("frag_helo_requires_damage_areas")?;
        let catalog = self.item_catalog.retail_combat()?;
        let mut blasts = Vec::new();
        let player = u32::from(actor.reservation.player);
        for terminal in terminals {
            let Some(position) = terminal
                .position
                .filter(|_| terminal.reason == ProjectileTerminalReason::ReportedDetonation)
            else {
                continue;
            };
            let frag = catalog
                .grenade_for_damage(terminal.item_id)
                .map_err(|_| "frag_helo_item_unavailable")?;
            let vehicle_hp = frag.damage.vehicle as f32;
            if vehicle_hp == 0. {
                continue;
            }
            blasts.push(HeloVehicleBlast {
                origin: [position.x, position.y, position.z].map(f64::from),
                projectile_id: ((16 + u32::from(terminal.slot)) << 24) | 0x0080_0000 | player,
                dose: HeloVehicleDose {
                    damage: TargetDamageParameters {
                        amount: vehicle_hp,
                        armor_pierce: 0.,
                        armor_redux: 0.,
                    },
                    half_radius_m: frag.half_radius as f32,
                    outer_radius_m: frag.full_radius as f32,
                    material_penetration: frag
                        .material_penetration
                        .ok_or("frag_helo_material_required")?
                        as f32,
                },
            });
        }
        let candidate = self.prepare_helo_vehicle_blasts(
            HeloBlastCredit {
                world: actor.reservation.world,
                attacker: 0x0380_0000 | player,
                team: actor.team(),
            },
            &blasts,
            HeloBlastFrame {
                rules,
                collision: world.frag_blast_collision(),
                owner_view: Some(owner_view),
            },
            now,
        )?;
        Ok(candidate.after.map(|fleet| PreparedGrenades {
            fleet,
            packets: candidate.packets,
            audit: candidate.audit,
        }))
    }

    fn prepare_helo_vehicle_blasts(
        &self,
        credit: HeloBlastCredit,
        blasts: &[HeloVehicleBlast],
        frame: HeloBlastFrame<'_>,
        now: Duration,
    ) -> Result<HeloBlastCandidate, &'static str> {
        if blasts.iter().any(|blast| {
            let dose = blast.dose;
            !blast
                .origin
                .map(|value| value as f32)
                .into_iter()
                .all(f32::is_finite)
                || dose.damage.amount <= 0.
                || ![
                    dose.damage.amount,
                    dose.damage.armor_pierce,
                    dose.damage.armor_redux,
                ]
                .into_iter()
                .all(|value| value.is_finite() && value >= 0.)
                || !dose.half_radius_m.is_finite()
                || !dose.outer_radius_m.is_finite()
                || dose.half_radius_m <= 0.
                || dose.outer_radius_m <= dose.half_radius_m
                || !dose.material_penetration.is_finite()
                || dose.material_penetration < 0.
        }) {
            return Err("invalid_hospital_rocket_helo_blast");
        }
        let mut candidate = HeloBlastCandidate::default();
        if blasts.is_empty() {
            return Ok(candidate);
        }
        let HeloBlastFrame {
            rules,
            collision,
            owner_view,
        } = frame;
        let HeloBlastCredit {
            world,
            attacker,
            team,
        } = credit;
        let state = &self
            .domination_gameplay
            .as_ref()
            .ok_or("hospital_rocket_domination_world_required")?
            .state;
        let fleet = state
            .helo
            .as_ref()
            .ok_or("domination_helo_world_required")?;
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
        let row = collision.filter_rows[6];
        let snapshot = match owner_view {
            Some(view) => self.owner_collision_snapshot(row, view)?,
            None => self.dynamic_collision_snapshot(row)?,
        };
        if snapshot.context().0.world_id != world {
            return Err("stale_hospital_rocket_helo_world");
        }
        let scene = self
            .collision_world
            .as_ref()
            .filter(|scene| {
                scene.stats().triangles != 0 && scene.metadata().map_name == self.map_name()
            })
            .ok_or("hospital_rocket_helo_static_collision_required")?;
        let mut overlap_budget = collision.limits.max_support_points;
        let mut static_budget = 1_000_000;
        let friendly = team == 0; // Helo belongs to the attacking team.
        let friendly_factor = if friendly {
            armor.friendly_fire_multiplier()
        } else {
            1.
        };
        let mut after = fleet.clone();
        let mut changed = BTreeSet::new();

        for &HeloVehicleBlast {
            origin,
            projectile_id,
            dose,
        } in blasts
        {
            let origin_f32 = origin.map(|axis| axis as f32);
            let sphere = Geometry::Sphere {
                radius: f64::from(dose.outer_radius_m),
            };
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
                    dose.damage,
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
                changed.insert(index);
                candidate.audit.push(json!({
                    "vehicle_index": index, "target": target, "location": location,
                    "projectile_id": projectile_id, "attacker": attacker,
                    "friendly": friendly, "radial_scale": scale, "amount_hp": amount,
                    "user_data": user_data, "health_before_quarter_points": before,
                    "health_after_quarter_points": wire.health_quarter_points,
                }));
            }
        }
        if !changed.is_empty() {
            candidate.packets = after
                .carriers
                .iter()
                .filter(|carrier| changed.contains(&carrier.source.object_index))
                .map(|carrier| {
                    damage_delta(
                        carrier.source.object_index,
                        None,
                        Some(carrier.damage.wire),
                        now,
                        self.wire_profile,
                    )
                })
                .collect::<Result<_, _>>()?;
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
            allocation_sequence: actor.allocation_sequence,
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

    pub(crate) fn frag_fixture(enabled: bool) -> Engine<Vec<u8>> {
        frag_fixture_with_team(enabled, 1)
    }

    fn frag_fixture_with_team(enabled: bool, source_team: u16) -> Engine<Vec<u8>> {
        let (mut engine, _) = fixture();
        engine.hospital_rockets = None;
        engine.projectile_contacts = None;
        let state = &mut engine.domination_gameplay.as_mut().unwrap().state;
        state.projectile_contacts = None;
        if enabled {
            state.helo_frag_damage_areas = Some(std::sync::Arc::new(Catalog::synthetic()));
        }
        engine.item_catalog = crate::human_kill_reward_tests::vehicle_grenade_items_with_cover(80.);
        let synthetic = crate::human_kill_reward_tests::fixture();
        let equipped = synthetic.active.as_ref().unwrap();
        let mut kit = equipped.spawned_loadout.clone().unwrap();
        kit.inventories[0].slots[4] = Some(4);
        let squad = engine
            .spawn_catalog
            .as_ref()
            .unwrap()
            .for_squad(source_team * 16)
            .unwrap();
        let actor = engine.active.as_mut().unwrap();
        actor.squad_catalog = std::sync::Arc::new(squad);
        actor.mounted_bunker = None;
        actor.mounted_seat = None;
        actor.current_inventory_handle = equipped.current_inventory_handle;
        actor.spawned_loadout = Some(kit);
        actor.grenade_remaining = 2;
        let launch = crate::hospital_rocket_runtime::tests::launch_wire(&engine, 3, 4);
        assert!(
            !crate::spawn_progression::dispatch(&mut engine, &launch, 1000)
                .unwrap()
                .is_empty()
        );
        assert!(engine.active.as_ref().unwrap().projectiles.get(3).is_some());
        engine
    }

    pub(crate) fn frag_terminal(engine: &Engine<Vec<u8>>, sequence: u32) -> Vec<u8> {
        let [x, y, z] = ORIGIN.map(|axis| axis as f32);
        projectile_replication::ProjectileOwnerDelta {
            header: replication::UpdateHeader {
                sequence,
                nak_sequence: 0,
                client_index: 0,
                object_type_mask: 1 << 19,
                timestamp_ms: 1100,
            },
            update: &projectile::ProjectileOwnerUpdate {
                slot: 3,
                object_index: engine.active.as_ref().unwrap().reservation.player,
                fields: projectile::ProjectileOwnerFields {
                    state: Some(2),
                    position: Some(CharacterPosition { x, y, z }),
                    rotation_xyzw: None,
                },
            },
        }
        .encode_with_profile(engine.wire_profile)
        .unwrap()
    }

    fn helo_wire(engine: &Engine<Vec<u8>>) -> replication::VehicleDamage {
        engine
            .domination_gameplay
            .as_ref()
            .unwrap()
            .state
            .helo
            .as_ref()
            .unwrap()
            .carriers[0]
            .damage
            .wire
    }

    #[test]
    fn ordinary_frag_helo_applies_classified_damage_without_other_opt_ins() {
        let mut engine = frag_fixture(true);
        let wire = frag_terminal(&engine, 200);
        crate::spawn_progression::dispatch(&mut engine, &wire, 1100).unwrap();
        let hit = helo_wire(&engine);
        assert_eq!(hit.last_location_and_type, 0xa2);
        assert_eq!(hit.health_quarter_points, 272); // (80 × .875 − 6) × .5 = 32 HP.
        assert_eq!(hit.last_attacker, 0x0380_0000);
        assert_eq!(hit.last_source_object, u32::MAX);
        assert_eq!(hit.last_user_data, (128 | 512) << 20);
    }

    #[test]
    fn ordinary_frag_helo_legacy_option_absent_preserves_area_zero() {
        let mut engine = frag_fixture(false);
        let wire = frag_terminal(&engine, 200);
        crate::spawn_progression::dispatch(&mut engine, &wire, 1100).unwrap();
        let hit = helo_wire(&engine);
        assert!(hit.health_quarter_points < 400);
        assert_eq!(hit.last_location_and_type, 0xa0);
        assert_eq!(hit.last_source_object, 0x1380_0000);
        assert_eq!(hit.last_user_data, 0);
    }

    #[test]
    fn ordinary_frag_helo_friendly_scaling_and_native_inbound_floor() {
        let mut friendly = frag_fixture_with_team(true, 0);
        let wire = frag_terminal(&friendly, 200);
        crate::spawn_progression::dispatch(&mut friendly, &wire, 1100).unwrap();
        // (80 × .875 × .25 − 6) × .5 = 5.75 HP.
        assert_eq!(helo_wire(&friendly).health_quarter_points, 377);
        assert_eq!(
            frag_event(&friendly)["decoded"]["helo_grenade_checks"][0]["friendly"],
            true
        );

        let mut inbound = frag_fixture(true);
        inbound
            .domination_gameplay
            .as_mut()
            .unwrap()
            .state
            .helo
            .as_mut()
            .unwrap()
            .carriers[0]
            .published_state = FlightState::Inbound;
        let wire = frag_terminal(&inbound, 200);
        crate::spawn_progression::dispatch(&mut inbound, &wire, 1100).unwrap();
        assert_eq!(helo_wire(&inbound).health_quarter_points, 360);
    }

    fn frag_event(engine: &Engine<Vec<u8>>) -> Value {
        engine
            .writer
            .split(|&byte| byte == b'\n')
            .rev()
            .filter_map(|row| serde_json::from_slice::<Value>(row).ok())
            .find(|row| row["op"] == "event")
            .unwrap()
    }

    fn frag_report(
        engine: &Engine<Vec<u8>>,
        sequence: u32,
        reports: &[(u8, CharacterPosition)],
        driver_position: Option<CharacterPosition>,
    ) -> Vec<u8> {
        let mut bits = replication::BitWriter::with_profile(engine.wire_profile);
        let player = engine.active.as_ref().unwrap().reservation.player;
        let mask = reports
            .iter()
            .fold(0, |mask, (slot, _)| mask | (1 << (16 + slot)));
        replication::UpdateHeader {
            sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: mask | (u32::from(driver_position.is_some()) << 1),
            timestamp_ms: 1100,
        }
        .encode(&mut bits)
        .unwrap();
        if let Some(position) = driver_position {
            bits.write(1, 1).unwrap();
            bits.write(0, 16).unwrap(); // Ground driver index0.
            bits.write(0, 25).unwrap();
            bits.write(1, 1).unwrap();
            position.encode(&mut bits).unwrap();
            bits.write(0, 5).unwrap();
        }
        for &(_, position) in reports {
            bits.write(1, 1).unwrap();
            bits.write(player.into(), 16).unwrap();
            projectile::ProjectileOwnerFields {
                state: Some(2),
                position: Some(position),
                rotation_xyzw: None,
            }
            .encode_fields(&mut bits)
            .unwrap();
            bits.write(0, 1).unwrap();
        }
        bits.finish()
    }

    #[test]
    fn ordinary_frag_helo_same_event_fuses_accumulate_and_emit_one_final_field18() {
        let mut engine = frag_fixture(true);
        engine.active.as_mut().unwrap().helo_creation_sent = true;
        let launch = crate::hospital_rocket_runtime::tests::launch_wire(&engine, 0, 4);
        assert!(
            !crate::spawn_progression::dispatch(&mut engine, &launch, 1000)
                .unwrap()
                .is_empty()
        );
        let wire = frag_report(
            &engine,
            200,
            &[
                (0, CharacterPosition { x: 3., ..POSE }),
                (3, CharacterPosition { z: 0., ..POSE }),
            ],
            None,
        );
        crate::spawn_progression::dispatch(&mut engine, &wire, 1100).unwrap();
        let record = frag_event(&engine);
        let doses = record["decoded"]["helo_grenade_checks"].as_array().unwrap();
        assert_eq!(doses.len(), 2);
        assert_eq!(doses[0]["projectile_id"], 0x1080_0000_u32);
        assert_eq!(doses[0]["health_before_quarter_points"], 400);
        assert_eq!(doses[0]["health_after_quarter_points"], 272);
        assert_eq!(doses[1]["projectile_id"], 0x1380_0000_u32);
        assert_eq!(doses[1]["health_before_quarter_points"], 272);
        assert_eq!(doses[1]["health_after_quarter_points"], 144);
        let damage = helo_wire(&engine);
        assert_eq!(damage.health_quarter_points, 144);
        assert_eq!(damage.last_location_and_type, 0xa5);
        assert_eq!(damage.last_user_data, 128); // Positive Z after the second terminal.
        let source = engine.active.as_ref().unwrap();
        let packets = source
            .pending_shared
            .iter()
            .filter(|wire| {
                ObjectUpdate::decode_with_profile(wire, engine.wire_profile)
                    .is_ok_and(|update| update.header.object_type_mask == 1 << 2)
            })
            .collect::<Vec<_>>();
        let [packet] = packets.as_slice() else {
            panic!("one final Helo delta expected")
        };
        let mut bits = BitReader::with_profile(packet, engine.wire_profile);
        bits.position = engine.wire_profile.header_bits();
        assert_eq!(bits.read(1), Ok(1));
        assert_eq!(bits.read(16), Ok(0));
        assert_eq!(bits.read(18), Ok(0));
        assert_eq!(bits.read(1), Ok(1));
        assert_eq!(bits.read(8), Ok(0xa5));
        assert_eq!(bits.read(16).unwrap() & 0x0fff, 144);
        assert_eq!(bits.read(32), Ok(0x0380_0000));
        assert_eq!(bits.read(32), Ok(u32::MAX));
        assert_eq!(bits.read(32), Ok(128));
    }

    #[test]
    fn ordinary_frag_helo_material_ray_sees_accepted_same_event_ground_motion() {
        use crate::dynamic_collision_runtime::{RuntimeBindings, VehicleBinding};
        let mut engine = frag_fixture_with_team(true, 0);
        let mut ground =
            crate::manual_ground_repair_domination_tests::fixture_with_drivers(0, 0, true);
        engine.ground_vehicles = ground.ground_vehicles.take();
        engine.ground_runtime = ground.ground_runtime.take();
        let mut bodies = crate::synthetic_dynamic_collision::fixture();
        for vertex in bodies["templates"][0]["roots"][0]["shape"]["vertices"]
            .as_array_mut()
            .unwrap()
        {
            vertex[0] = json!(vertex[0].as_f64().unwrap() * 0.125);
        }
        let active = engine.active.take();
        let peers = std::mem::take(&mut engine.peers);
        engine = engine
            .with_dynamic_collision_bodies(
                dynamic_collision::Catalog::from_json(&bodies.to_string()).unwrap(),
                RuntimeBindings {
                    character_template: "invented-character".to_owned(),
                    vehicles: [(110, 0), (111, 1)]
                        .map(|(vehicle_type_token, faction)| VehicleBinding {
                            vehicle_type_token,
                            faction,
                            template: "invented-vehicle".to_owned(),
                        })
                        .into(),
                },
            )
            .unwrap();
        engine.active = active;
        engine.peers = peers;
        let source = engine.active.as_mut().unwrap();
        source.ground_vehicle_batch = engine.ground_vehicles.as_ref().unwrap().batch_count();
        source.ground_seat = Some(ground_seats::PassengerSeat {
            vehicle_index: 0,
            seat: 0,
        });
        let ground = engine.ground_runtime.as_mut().unwrap();
        let cover = CharacterPosition { x: 2.5, ..POSE };
        let key = ground.get(0).unwrap().key;
        ground
            .force_pose(
                key,
                ground::Pose {
                    position: cover,
                    rotation: [0., 0., 0., 1.],
                },
            )
            .unwrap();
        ground.begin_driver(0, 1000).unwrap();
        let state = &mut engine.domination_gameplay.as_mut().unwrap().state;
        let profiles = [(110, 0), (111, 1)].map(|(token, faction)| {
            json!({
                "vehicle_type_token":token,"faction":faction,"max_health":150.,
                "critical_loss_percent_per_second":0.,
                "armor":{"rating":1000.,"prevention":0.,"coverage":[0.,0.,0.,0.]}
            })
        });
        state.ground_damage = std::sync::Arc::new(
            serde_json::from_value(json!({
                "friendly_fire_multiplier":0.5,"profiles":profiles,
                "areas":vec![json!({"armor_area":0,"multiplier":1.});16]
            }))
            .unwrap(),
        );
        let rows = state.filter_rows;
        let snapshot = engine.dynamic_collision_snapshot(rows[6]).unwrap();
        assert!(
            snapshot
                .positive_ray_hit_ignoring(
                    ORIGIN,
                    [2., 2.5, 1.],
                    [None; 2],
                    rows,
                    Default::default(),
                )
                .unwrap()
        );

        let moved = CharacterPosition { z: 3., ..cover };
        let wire = frag_report(
            &engine,
            200,
            &[(3, CharacterPosition { x: 3., ..POSE })],
            Some(moved),
        );
        crate::spawn_progression::dispatch(&mut engine, &wire, 1100).unwrap();
        assert_eq!(engine.current_ground_vehicle(0).unwrap().position, moved);
        assert_eq!(
            helo_wire(&engine).health_quarter_points,
            377,
            "{}",
            frag_event(&engine)
        );
    }

    #[test]
    fn ordinary_frag_helo_query_refusal_keeps_the_fuse_while_legacy_errors_remain() {
        for (dynamic_missing, error) in [
            (true, "dynamic_body_resources_unavailable"),
            (false, "hospital_rocket_helo_mesh_required"),
        ] {
            let mut engine = frag_fixture(true);
            if dynamic_missing {
                engine.dynamic_body_resources = None;
            } else {
                engine
                    .domination_gameplay
                    .as_mut()
                    .unwrap()
                    .state
                    .collisions
                    .remove_helo(0);
            }
            let wire = frag_terminal(&engine, 200);
            crate::spawn_progression::dispatch(&mut engine, &wire, 1100).unwrap();
            let source = engine.active.as_ref().unwrap();
            assert!(source.projectiles.get(3).unwrap().completed);
            assert!(engine.projectile_cleanup_pending(source, 3));
            assert_eq!(helo_wire(&engine).health_quarter_points, 400);
            let audit = frag_event(&engine);
            assert_eq!(audit["decoded"]["helo_grenade_checks"][0]["error"], error);
            if dynamic_missing {
                assert_eq!(audit["decoded"]["ground_grenade_checks"][0]["error"], error);
            }
        }
        let mut legacy = frag_fixture(false);
        legacy.dynamic_body_resources = None;
        let wire = frag_terminal(&legacy, 200);
        assert_eq!(
            crate::spawn_progression::dispatch(&mut legacy, &wire, 1100).unwrap_err(),
            "dynamic_body_resources_unavailable"
        );
        assert!(
            !legacy
                .active
                .as_ref()
                .unwrap()
                .projectiles
                .get(3)
                .unwrap()
                .completed
        );
    }

    #[test]
    fn ordinary_frag_helo_rejects_stale_terminal_and_rolls_back_declined_record() {
        let mut engine = frag_fixture(true);
        engine.active.as_mut().unwrap().latest_owner_sequence = 200;
        let stale = frag_terminal(&engine, 200);
        crate::spawn_progression::dispatch(&mut engine, &stale, 1100).unwrap();
        assert_eq!(helo_wire(&engine).health_quarter_points, 400);
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
        let fresh = frag_terminal(&engine, 201);
        engine.max_log = engine.written;
        assert_eq!(
            crate::spawn_progression::dispatch(&mut engine, &fresh, 1101).unwrap_err(),
            "log_limit_reached"
        );
        assert_eq!(helo_wire(&engine).health_quarter_points, 400);
        let source = engine.active.as_ref().unwrap();
        assert!(!source.projectiles.get(3).unwrap().completed);
        assert!(!engine.projectile_cleanup_pending(source, 3));
    }

    fn candidate(
        engine: &Engine<Vec<u8>>,
        source: &LaunchSource,
    ) -> Result<HeloBlastCandidate, &'static str> {
        candidate_with_dose(engine, source, DOSE.into())
    }

    fn candidate_with_dose(
        engine: &Engine<Vec<u8>>,
        source: &LaunchSource,
        dose: HeloVehicleDose,
    ) -> Result<HeloBlastCandidate, &'static str> {
        let state = &engine.domination_gameplay.as_ref().unwrap().state;
        engine.prepare_hospital_rocket_helo_blast(
            source,
            ORIGIN,
            dose,
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
    fn underbarrel_helo_vehicle_blast_preserves_the_typed_armor_reduction() {
        let (engine, source) = fixture();
        let legacy = candidate(&engine, &source).unwrap();
        let reduced = candidate_with_dose(
            &engine,
            &source,
            HeloVehicleDose::equipment(
                TargetDamageParameters {
                    amount: DOSE.vehicle_hp,
                    armor_pierce: 0.,
                    armor_redux: 0.1,
                },
                DOSE,
            ),
        )
        .unwrap();
        assert_eq!(legacy.audit[0]["amount_hp"], 27.);
        let amount = reduced.audit[0]["amount_hp"].as_f64().unwrap();
        assert!((amount - 32.4).abs() < 0.001);
        let wire = reduced.after.unwrap().carriers[0].damage.wire;
        assert!(wire.health_quarter_points < 292);
        assert_eq!(wire.last_location_and_type, 0xa2);
        assert_eq!(wire.last_source_object, u32::MAX);
        assert_eq!(helo_wire(&engine).health_quarter_points, 400);
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
