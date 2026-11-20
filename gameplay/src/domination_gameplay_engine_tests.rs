use super::*;
use crate::{acquisition_round::Rules, protocol};
use serde_json::json;

const FSP: u32 = 203;
const FIRST: u32 = 10;
const NEXT: u32 = 11;

fn test_time(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

fn fixture() -> (Runtime, ObjectiveWorld, SpawnCatalog, Round) {
    let hash = "1".repeat(64);
    let squads: Vec<_> = (0..32)
        .map(|s| {
            json!({"index":s,"team":s/16,"platoon":s%16/4,"squad":s%4,
            "faction":if s>=16 { Some(0) } else { None },"available":[s/16],"default_spawn":s/16})
        })
        .collect();
    let orders: Vec<_> = (0..32)
        .map(|s| {
            json!({"squad_index":s,"team":s/16,"initial_objective_index":65535,
            "authored_objectives":[{"object_index":0,"order":1},{"object_index":1,"order":2}]})
        })
        .collect();
    let source = json!({"level_archive_sha256":hash,"level_clv_sha256":hash}).to_string();
    let spawns = SpawnCatalog::from_json(&json!({
        "format":"mag_spawn_catalog","schema_version":1,"source":source,
        "active":{"map_name":"synthetic","squad_index":0,"default_spawn":0,"available":[0]},
        "match_profile":{"game_type":2,"auto_assign_teams":true,"team_capacity":16,"squad_capacity":1,
            "attacking_faction":1,"has_bunkers":false,"has_objectives":true},
        "maps":[{"name":"synthetic","owner_faction":0,"spawn_count":3,
            "runtime_indices_verified":true,"runtime_indices_basis":"SYNTHETIC slot proof","squads":squads,
            "locations":[
                {"index":0,"token":201,"kind":"insertion","position":{"x":1.,"y":2.,"z":1.}},
                {"index":1,"token":202,"kind":"insertion","position":{"x":2.,"y":2.,"z":1.}},
                {"index":2,"token":FSP,"kind":"on_foot","active":false,"position":{"x":3.,"y":2.,"z":1.}}
            ],
        }],
    }).to_string()).unwrap();
    let startup_raw = json!({
        "schema":"mag_domination_mission_start_requests_v1",
        "status":"ORIGINAL_AUTHORED_AND_STATIC_NATIVE_STARTUP_REQUESTS_BOUND",
        "source":{"authored_metadata_sha256":hash,"native_program_sha256":hash},
        "maps":[{"map_name":"synthetic","source":{"archive_sha256":hash,"clv_sha256":hash},
            "objectives":[
                {"token":"0x0000000a","client_network_index":0,"property_class":"ObjectiveDemolitionStructuralProperties",
                    "property_active":{"value":true},"saved_desired_state":1,"transition_request":1,
                    "current_network_state_after_mission_start":null,"sim_network_index":null},
                {"token":"0x0000000b","client_network_index":1,"property_class":"ObjectiveSecureAndHoldProperties",
                    "property_active":{"value":true},"saved_desired_state":0,"transition_request":0,
                    "current_network_state_after_mission_start":null,"sim_network_index":null}
            ],
        }],
    }).to_string().into_bytes();
    let objectives = Arc::new(ObjectiveCatalog::from_json(&json!({
        "format":"mag_retail_objective_catalog","schema_version":1,"application_id":20231,
        "protocol":protocol::ProtocolInfo::RETAIL_PROTOCOL,"game_type":2,"map_name":"synthetic",
        "entity_family":12,"network_type":12,
        "source":{"authored_catalog_sha256":hash,"binding_capture_sha256":hash,
            "startup_proof_sha256":format!("{:x}", Sha256::digest(&startup_raw)),"source_image_sha256":hash,
            "capture_qualification":"native_source_gameplay"},
        "objects":[
            {"token":"0x0000000a","object_index":0,"initial_state":0,"native_class":"structural",
             "demolition":{"owner_team":1,"arm_ms":10,"detonate_ms":30,"defuse_ms":10,
                "can_defuse":true,"activation_score":0,"deactivation_score":0,
                "position":[1.,2.,1.],"trigger_count":1,"maximum_distance_m":5.,
                "distance_policy":"authored_trigger_box","trigger_volumes":[
                    {"centre":[1.,2.,1.],"axes":[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]],"half_extents":[1.,1.,1.]}
                ]}},
            {"token":"0x0000000b","object_index":1,"initial_state":0,"native_class":"secure_hold",
             "interaction":{"owner_team":1,"activate_ms":37,"complete_ms":43,"deactivate_ms":47,
                 "can_deactivate":true,"position":[4.,2.,1.],"trigger_count":1,"maximum_distance_m":5.,
                 "distance_policy":"authored_trigger_box","trigger_volumes":[
                     {"centre":[4.,2.,1.],"axes":[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]],"half_extents":[1.,1.,1.]}
                 ]}}
        ], "squads":orders,
    }).to_string()).unwrap());
    let controller = domination_control::Catalog::from_json(json!({
        "format":"mag_domination_control","schema_version":1,"game_archive_sha256":hash,
        "metadata_sha256":hash,"script_sha256":[hash],
        "maps":[{"name":"synthetic","level_archive_sha256":hash,"level_clv_sha256":hash,
            "bindings":[{"binding_token":77,"entries":[
                {"delay_ms":0,"rules":[{"conditions":[],"commands":[
                    {"kind":"set_state","objective_token":NEXT,"state":"completed","caller":"none"},
                    {"kind":"score_rate","team":1,"amount":1,"frequency_ms":8,"objectives":[]},
                    {"kind":"ability","platoon_token":31,"ability":"sensor","enabled":true}
                ]}]},
                {"delay_ms":5,"rules":[{
                    "conditions":[{"objective_token":NEXT,"state":"completed","equal":true}],
                    "commands":[{"kind":"ability","platoon_token":31,"ability":"sensor","enabled":false}]
                }]}
            ]}],
            "hooks":[{"objective_token":FIRST,"event":"deactivated","binding_token":77,"resolved":true}],
        }],
    }).to_string().as_bytes()).unwrap();
    let full_squads: Vec<_> = (0..32)
        .map(|index| {
            let (points, base) = if index < 16 {
                (vec![201], 201)
            } else {
                (vec![202, FSP], 202)
            };
            json!({"index":index,"points":points,"category_zero":base})
        })
        .collect();
    let fsp_raw = json!({
        "format":"mag_domination_fsp_bindings_v1","metadata_sha256":hash,
        "spawn_catalog_sha256":spawns.source_sha256().unwrap(),"map_name":"synthetic",
        "points":[{"token":201,"index":0,"team":0,"user_class":0},
                  {"token":202,"index":1,"team":1,"user_class":0},
                  {"token":FSP,"index":2,"team":1,"user_class":0}],
        "links":[{"objective_token":FIRST,"spawn_token":FSP,"fsp_type":0}],
        "squads":full_squads,
    })
    .to_string()
    .into_bytes();
    let schedule = json!({
        "schema":"mag_domination_structural_sim_schedule_v1",
        "status":"RETAIL_SIM_SOURCE_ORDER_AND_COMPLETION_PREDICATE_BOUND",
        "retail_elf_sha256":hash,"metadata_sha256":hash,"client_source_bindings_sha256":hash,
        "completed_predicate":[5,7],
        "maps":[{"map":"synthetic","ordered_tokens":["0x0000000a","0x0000000b"]}],
    })
    .to_string();
    let setup = Setup::from_sources(
        objectives.clone(),
        &spawns,
        &controller,
        &startup_raw,
        schedule.as_bytes(),
        &fsp_raw,
    )
    .unwrap();
    let runtime = Runtime::new(Arc::new(setup), NonZeroUsize::new(8).unwrap());
    let world = ObjectiveWorld::new(&objectives);
    let round = Round::new(Rules {
        starting_points: [61, 79],
        major_percent: 36,
        end_delay_ms: 200,
    });
    (runtime, world, spawns, round)
}

#[derive(Default)]
struct Spy {
    calls: Vec<(&'static str, u16)>,
    fail_ability: bool,
    fail_sequence: bool,
    allow_strike_lifecycle: bool,
    precision_trigger: bool,
}

impl ExternalEffects for Spy {
    fn prepare_class_prefix(
        &mut self,
        entry: NativeEntry,
        _: &ObjectiveWorld,
        spawns: &SpawnCatalog,
    ) -> Result<(), &'static str> {
        if entry.delta.object_index == 0 && !self.allow_strike_lifecycle {
            assert!(!spawns.point_active(2));
        }
        self.calls.push(("class_prefix", entry.delta.object_index));
        Ok(())
    }

    fn prepare_vehicles(&mut self, requests: &[VehicleRequest]) -> Result<(), &'static str> {
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].point.token, FSP);
        self.calls.push(("vehicle", 0));
        Ok(())
    }

    fn prepare_class_entry(
        &mut self,
        entry: NativeEntry,
        world: &ObjectiveWorld,
        spawns: &SpawnCatalog,
    ) -> Result<(), &'static str> {
        if !self.allow_strike_lifecycle {
            assert!(spawns.point_active(2));
        }
        assert_eq!(
            world.state(entry.delta.object_index),
            Some(entry.delta.state_actor.state)
        );
        self.calls.push(("class_suffix", entry.delta.object_index));
        Ok(())
    }

    fn prepare_explosion(
        &mut self,
        entry: NativeEntry,
        _: &ObjectiveWorld,
    ) -> Result<(), &'static str> {
        if !self.allow_strike_lifecycle {
            return Err("unexpected_explosion");
        }
        assert_eq!(
            entry.active_trigger_index,
            (!self.precision_trigger).then_some(0)
        );
        self.calls.push(("explosion", entry.delta.object_index));
        Ok(())
    }

    fn prepare_ability(
        &mut self,
        context: Context,
        platoon: u32,
        ability: Ability,
        enabled: bool,
    ) -> Result<(), &'static str> {
        assert_eq!(context.transition.objective_token, FIRST);
        assert_eq!(context.actor_object_id, u32::MAX);
        assert_eq!(platoon, 31);
        assert_eq!(ability, Ability::Sensor);
        self.calls
            .push((if enabled { "immediate" } else { "delayed" }, 0));
        if self.fail_ability {
            Err("synthetic_ability_failure")
        } else {
            Ok(())
        }
    }

    fn original_actor_id(&self, player: u16, generation: u32) -> Result<u32, &'static str> {
        if (player, generation) != (3, 5) {
            return Err("unexpected_actor");
        }
        Ok(0x0380_0000 | u32::from(player))
    }

    fn callback_actor(&self, _: u32) -> Result<Option<Actor>, &'static str> {
        Err("unexpected_callback_actor")
    }

    fn refresh_assignments(
        &mut self,
        entry: NativeEntry,
        _: &ObjectiveCatalog,
        _: &ObjectiveWorld,
        _: &SpawnCatalog,
    ) -> Result<(), &'static str> {
        self.calls.push(("assignment", entry.delta.object_index));
        Ok(())
    }
    fn prepare_sequence(
        &mut self,
        entry: NativeEntry,
        token: u32,
        now: u32,
        _: &ObjectiveWorld,
    ) -> Result<Option<ObjectiveSequenceDelta>, &'static str> {
        assert_eq!(token, FIRST + u32::from(entry.delta.object_index));
        self.calls.push(("sequence", entry.delta.object_index));
        if self.fail_sequence {
            return Err("synthetic_sequence_failure");
        }
        Ok(Some(ObjectiveSequenceDelta {
            object_index: entry.delta.object_index,
            sequence_index: i32::from(entry.delta.object_index) - 1,
            start_time_ms: now,
        }))
    }
}

fn frame<'a>(
    ms: u64,
    world: &'a ObjectiveWorld,
    spawns: &'a SpawnCatalog,
    round: &'a Round,
) -> Frame<'a> {
    Frame {
        now: test_time(ms),
        world,
        spawns,
        round,
    }
}

#[test]
fn staged_mission_uses_committed_entries_and_preserves_qualified_order() {
    let (runtime, world, spawns, round) = fixture();
    let started = runtime
        .stage_mission_start(frame(100, &world, &spawns, &round), Spy::default())
        .unwrap();
    assert_eq!(started.world.state(0), Some(ObjectiveState::Inactive));
    assert!(started.deltas.is_empty());
    let accepted = started
        .runtime
        .stage_objective_update(
            frame(100, &started.world, &started.spawns, &started.round),
            &BTreeMap::new(),
            started.effects,
        )
        .unwrap();
    assert_eq!(accepted.world.state(0), Some(ObjectiveState::Deactivated));
    assert_eq!(accepted.world.state(1), Some(ObjectiveState::Completed));
    assert_eq!(accepted.spawns.default_for_squad(31), Some(2));
    assert_eq!(
        accepted.effects.calls,
        [
            ("class_prefix", 0),
            ("vehicle", 0),
            ("class_suffix", 0),
            ("immediate", 0),
            ("assignment", 0),
            ("sequence", 0),
            ("class_prefix", 1),
            ("class_suffix", 1),
            ("assignment", 1),
            ("sequence", 1),
        ]
    );
    let profiles = crate::wire_profile::WireProfileCatalog::embedded();
    let profile = profiles.profile(20231).unwrap();
    let packet = accepted.objective_packet(3, 100, profile).unwrap().unwrap();
    let header = crate::replication::ObjectUpdate::decode_with_profile(&packet, profile)
        .unwrap()
        .header;
    assert_eq!(header.object_type_mask, 1 << 12);
    let mut bits = crate::replication::BitReader::with_profile(&packet, profile);
    bits.position = profile.header_bits();
    assert_eq!(bits.read(1), Ok(1));
    assert_eq!(bits.read(16), Ok(0));
    for field in 0..11 {
        let present = matches!(field, 2 | 3 | 4 | 6 | 9 | 10);
        assert_eq!(bits.read(1), Ok(u32::from(present)));
        match field {
            2 => {
                assert_eq!(bits.read(8), Ok(ObjectiveState::Deactivated as u32));
                assert_eq!(bits.read(32), Ok(100));
                assert_eq!(bits.read(16), Ok(u32::from(u16::MAX)));
            }
            3 | 4 => assert_eq!(bits.read(32), Ok(0)),
            6 => assert_eq!(bits.read(8), Ok(100)),
            9 => assert_eq!(bits.read(32), Ok(u32::MAX)),
            10 => assert_eq!(bits.read(32), Ok(100)),
            _ => {}
        }
    }
    assert_eq!(accepted.round.points(), round.points());
    let mut clock = accepted.round.clone();
    assert_eq!(clock.update_bleed(109), [None, Some(78)]);

    let too_soon = accepted
        .runtime
        .stage_callback_update(
            frame(104, &accepted.world, &accepted.spawns, &accepted.round),
            Spy::default(),
        )
        .unwrap();
    assert!(too_soon.effects.calls.is_empty());
    let delayed = too_soon
        .runtime
        .stage_callback_update(
            frame(105, &too_soon.world, &too_soon.spawns, &too_soon.round),
            Spy::default(),
        )
        .unwrap();
    assert_eq!(delayed.effects.calls, [("delayed", 0)]);
    let replay = delayed
        .runtime
        .stage_callback_update(
            frame(105, &delayed.world, &delayed.spawns, &delayed.round),
            Spy::default(),
        )
        .unwrap();
    assert!(replay.effects.calls.is_empty());
}

#[test]
fn missing_external_effect_refuses_candidate_without_mutating_initial_authority() {
    let (runtime, world, spawns, round) = fixture();
    let started = runtime
        .stage_mission_start(frame(100, &world, &spawns, &round), Spy::default())
        .unwrap();
    let result = started.runtime.stage_objective_update(
        frame(100, &started.world, &started.spawns, &started.round),
        &BTreeMap::new(),
        Spy {
            fail_ability: true,
            ..Default::default()
        },
    );
    assert_eq!(result.err(), Some("synthetic_ability_failure"));
    assert!(!started.spawns.point_active(2));
    assert_eq!(started.world.state(0), Some(ObjectiveState::Inactive));
    assert_eq!(started.world.state(1), Some(ObjectiveState::Inactive));
    assert_eq!(started.runtime.callbacks.pending_count(), 0);
    let mut clock = started.round.clone();
    assert_eq!(clock.update_bleed(109), [None, None]);

    let retried = started
        .runtime
        .stage_objective_update(
            frame(100, &started.world, &started.spawns, &started.round),
            &BTreeMap::new(),
            Spy::default(),
        )
        .unwrap();
    assert_eq!(retried.world.state(1), Some(ObjectiveState::Completed));
    assert_eq!(retried.runtime.callbacks.pending_count(), 1);
}

#[test]
fn player_start_and_release_commit_synchronously_with_reply_and_provider_guard() {
    use ObjectiveInteractionKind::{Ack, Deactivate, Nak, Release};
    let (runtime, world, spawns, round) = fixture();
    let started = runtime
        .stage_mission_start(frame(100, &world, &spawns, &round), Spy::default())
        .unwrap();
    let root = started
        .runtime
        .stage_objective_update(
            frame(100, &started.world, &started.spawns, &started.round),
            &BTreeMap::new(),
            started.effects,
        )
        .unwrap();
    assert_eq!(root.world.state(0), Some(ObjectiveState::Deactivated));
    assert_eq!(root.world.state(1), Some(ObjectiveState::Completed));
    let actor = ObjectiveActor {
        player: 3,
        team: 1,
        generation: 5,
        position: crate::replication::CharacterPosition {
            x: 4.,
            y: 2.,
            z: 1.,
        },
    };
    let command = |kind, player, trigger| PlayerInteraction {
        request: ObjectiveInteraction {
            target_object_id: 0x0c80_0001,
            kind,
            actor_index: player,
            trigger_index: trigger,
        },
        actor,
        timer_modifier: 0.,
    };
    let wrong_actor = root
        .runtime
        .stage_interaction(
            frame(101, &root.world, &root.spawns, &root.round),
            command(Deactivate, 99, 0),
            Spy::default(),
        )
        .unwrap();
    assert_eq!(wrong_actor.reply.kind, Nak);
    assert!(wrong_actor.candidate.deltas.is_empty());

    let armed = root
        .runtime
        .stage_interaction(
            frame(101, &root.world, &root.spawns, &root.round),
            command(Deactivate, 3, 0),
            Spy::default(),
        )
        .unwrap();
    assert_eq!(armed.reply.kind, Ack);
    assert_eq!(armed.reply.trigger_index, 0);
    assert_eq!(
        armed.candidate.world.state(1),
        Some(ObjectiveState::Retaking)
    );
    assert_eq!(armed.candidate.deltas.len(), 1);
    assert_eq!(root.world.state(1), Some(ObjectiveState::Completed));
    assert_eq!(armed.candidate.runtime.callbacks.pending_count(), 1);

    let prior = armed.candidate;
    let failed = prior.runtime.stage_interaction(
        frame(102, &prior.world, &prior.spawns, &prior.round),
        command(Release, 3, u8::MAX),
        Spy {
            fail_sequence: true,
            ..Spy::default()
        },
    );
    assert_eq!(failed.err(), Some("synthetic_sequence_failure"));
    assert_eq!(prior.world.state(1), Some(ObjectiveState::Retaking));
    let released = prior
        .runtime
        .stage_interaction(
            frame(102, &prior.world, &prior.spawns, &prior.round),
            command(Release, 3, u8::MAX),
            Spy::default(),
        )
        .unwrap();
    assert_eq!(released.reply.kind, Ack);
    assert_eq!(released.reply.trigger_index, u8::MAX);
    assert_eq!(
        released.candidate.world.state(1),
        Some(ObjectiveState::Completed)
    );
    assert_eq!(
        released.candidate.world.state(0),
        Some(ObjectiveState::Deactivated)
    );
}

#[test]
fn engine_records_startup_before_installing_objective_requests() {
    let (mut runtime, world, spawns, round) = fixture();
    let mut engine = crate::human_kill_reward_tests::fixture();
    engine.objective_world = Some(world);
    engine.spawn_catalog = Some(spawns);
    engine.domination_round = Some(round);
    let original_revision = engine.sequence;
    engine.failed = true;
    assert_eq!(
        engine
            .commit_domination_mission_start(&mut runtime, test_time(1), 0)
            .unwrap_err(),
        "recorder_unavailable"
    );
    assert_eq!(engine.sequence, original_revision);
    assert!(!runtime.started);
    engine.failed = false;
    engine
        .commit_domination_mission_start(&mut runtime, test_time(1), 0)
        .unwrap();
    assert!(runtime.started);
    assert!(engine.sequence > original_revision);
    assert_eq!(
        engine.objective_world.as_ref().unwrap().state(0),
        Some(ObjectiveState::Inactive)
    );
}

#[test]
fn strike_entry_stages_existing_effects_and_progresses_without_a_held_actor() {
    use ObjectiveState::{Activated, Activating, Completed};

    let (runtime, world, spawns, round) = fixture();
    let started = runtime
        .stage_mission_start(frame(100, &world, &spawns, &round), Spy::default())
        .unwrap();
    let mut live = started
        .runtime
        .stage_objective_update(
            frame(100, &started.world, &started.spawns, &started.round),
            &BTreeMap::new(),
            started.effects,
        )
        .unwrap();

    let source = crate::structure_damage::Source {
        player: 3,
        generation: 5,
    };
    let transition = live
        .world
        .activate_destroyed_trigger(0, 0, source, test_time(101), -0.5)
        .unwrap()
        .unwrap();
    let owned_effects = || Spy {
        allow_strike_lifecycle: true,
        ..Spy::default()
    };
    assert_eq!(
        live.runtime
            .stage_objective_update(
                frame(101, &live.world, &live.spawns, &live.round),
                &BTreeMap::new(),
                owned_effects(),
            )
            .err(),
        Some("domination_gameplay_unconsumed_native_entries"),
    );
    assert_eq!(
        live.runtime
            .stage_structure_damage_entries(
                frame(101, &live.world, &live.spawns, &live.round),
                Spy {
                    fail_sequence: true,
                    ..owned_effects()
                },
            )
            .err(),
        Some("synthetic_sequence_failure"),
    );
    let strike = live
        .runtime
        .stage_structure_damage_entries(
            frame(101, &live.world, &live.spawns, &live.round),
            owned_effects(),
        )
        .unwrap();
    assert_eq!(strike.deltas, [transition]);
    assert_eq!(strike.world.state(0), Some(Activating));
    assert_eq!(strike.sequences.len(), 1);
    assert_eq!(
        strike.effects.calls,
        [
            ("class_prefix", 0),
            ("class_suffix", 0),
            ("assignment", 0),
            ("sequence", 0)
        ]
    );
    let empty = strike
        .runtime
        .stage_structure_damage_entries(
            frame(101, &strike.world, &strike.spawns, &strike.round),
            owned_effects(),
        )
        .unwrap();
    assert!(empty.deltas.is_empty());
    assert!(empty.effects.calls.is_empty());

    let armed = empty
        .runtime
        .stage_objective_update(
            frame(106, &empty.world, &empty.spawns, &empty.round),
            &BTreeMap::new(),
            empty.effects,
        )
        .unwrap();
    assert_eq!(armed.world.state(0), Some(Activated));
    assert_eq!(armed.deltas[0].state_actor.actor_index, source.player);
    let completed = armed
        .runtime
        .stage_objective_update(
            frame(136, &armed.world, &armed.spawns, &armed.round),
            &BTreeMap::new(),
            armed.effects,
        )
        .unwrap();
    assert_eq!(completed.world.state(0), Some(Completed));
    assert_eq!(completed.deltas[0].state_actor.actor_index, source.player);
    assert!(completed.effects.calls.contains(&("explosion", 0)));
}

#[test]
fn precision_completion_uses_the_normal_entry_and_rolls_back_on_effect_failure() {
    let (runtime, world, spawns, round) = fixture();
    let started = runtime
        .stage_mission_start(frame(100, &world, &spawns, &round), Spy::default())
        .unwrap();
    let mut live = started
        .runtime
        .stage_objective_update(
            frame(100, &started.world, &started.spawns, &started.round),
            &BTreeMap::new(),
            started.effects,
        )
        .unwrap();
    assert!(
        live.world
            .stage_precision_strike(
                0,
                Some(crate::structure_damage::Source {
                    player: 3,
                    generation: 5
                }),
            )
            .unwrap()
    );

    let spy = |fail_sequence| Spy {
        fail_sequence,
        allow_strike_lifecycle: true,
        precision_trigger: true,
        ..Spy::default()
    };
    let evaluate = |fail_sequence| {
        live.runtime.stage_objective_update(
            frame(101, &live.world, &live.spawns, &live.round),
            &BTreeMap::new(),
            spy(fail_sequence),
        )
    };
    assert_eq!(evaluate(true).err(), Some("synthetic_sequence_failure"));
    assert_eq!(live.world.state(0), Some(ObjectiveState::Deactivated));
    let candidate = evaluate(false).unwrap();
    assert_eq!(candidate.world.state(0), Some(ObjectiveState::Completed));
    assert_eq!(candidate.deltas[0].state_actor.actor_index, 3);
    assert!(!candidate.spawns.point_active(2));
    assert_eq!(candidate.deltas.len(), 1);
    assert_eq!(candidate.sequences.len(), 1);
    assert_eq!(
        candidate.effects.calls,
        [
            ("class_prefix", 0),
            ("class_suffix", 0),
            ("assignment", 0),
            ("sequence", 0),
            ("class_prefix", 0),
            ("class_suffix", 0),
            ("assignment", 0),
            ("sequence", 0),
            ("class_prefix", 0),
            ("vehicle", 0),
            ("class_suffix", 0),
            ("explosion", 0),
            ("assignment", 0),
            ("sequence", 0),
        ]
    );
}
