//! Whole world updates through command bombardments with invented arenas.
//! A bomblet over a placed structure, friendly bots, the invoker, enemies or
//! empty ground must never fail the world update, and one failing effect must
//! end cleanly without leaving any of its staged work behind.
use crate::*;
use crate::{
    active_abilities::Ability,
    active_ability_effects::{ClusterRules, EffectRules},
    active_ability_engine_tests::strike_floor,
};
use std::{sync::Arc, time::Duration};

pub(crate) const LEADER: u16 = 0;
const FRIENDLY_KILLS: u8 = 14;

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

pub(crate) fn events(engine: &Engine<Vec<u8>>) -> Vec<Value> {
    std::str::from_utf8(&engine.writer)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Every staged row of a committed world record, whatever its mode names it.
pub(crate) fn staged_rows(engine: &Engine<Vec<u8>>) -> Vec<Value> {
    events(engine)
        .into_iter()
        .flat_map(|event| {
            ["character_effects", "effects"]
                .into_iter()
                .filter_map(|field| event[field].as_array().cloned())
                .flatten()
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Thirty wide bomblets, so one bombardment lands on characters, on the
/// structure and on bare ground.
pub(crate) fn wide_cluster() -> EffectRules {
    EffectRules::Cluster(ClusterRules {
        delay_ms: 100,
        damage_delay_ms: 50,
        interval_ms: 100,
        count: 30,
        scatter_radius: 9.,
        damage: 150.,
        inner_radius: 2.,
        inner_scale: 0.5,
        outer_radius: 5.,
        outer_scale: 0.,
    })
}

/// Install the invented command runtime, arena floor, character bodies,
/// friendly-fire rules and live combat loadouts on a running world.
pub(crate) fn arm_world(engine: &mut Engine<Vec<u8>>, rows: &mut [u32; 32]) {
    let mut setup = crate::active_ability_engine_tests::setup();
    setup.map_name = engine.map_name().to_owned();
    for row in &mut setup.effects {
        if row.2 == Ability::ClusterBomb {
            row.3 = wide_cluster();
        }
    }
    engine.active_abilities = Some(Arc::new(setup).runtime().unwrap());
    engine.collision_world = Some(strike_floor(engine.map_name(), true));
    let bodies = crate::active_ability_engine_tests::configured()
        .with_dynamic_collision_bodies(
            dynamic_collision::Catalog::from_json(
                &crate::synthetic_dynamic_collision::fixture().to_string(),
            )
            .unwrap(),
            dynamic_collision_runtime::RuntimeBindings {
                character_template: "invented-character".into(),
                vehicles: Vec::new(),
            },
        )
        .unwrap();
    if engine.dynamic_body_resources.is_none() {
        engine.dynamic_body_resources = bodies.dynamic_body_resources;
    }
    let spawn_sha256 = engine
        .spawn_catalog
        .as_ref()
        .and_then(|catalog| catalog.source_sha256().map(str::to_owned))
        .unwrap_or_else(|| "d".repeat(64));
    engine.character_damage_rules = Some(
        character_damage_rules::Rules::from_json(
            &json!({"schema_version":1,"application_id":20231,"map_name":engine.map_name(),
                "game_type":engine.match_profile().game_type,"friendly_fire_multiplier":0.5,
                "provenance":{"spawn_catalog_sha256":spawn_sha256,"source":"synthetic"}})
            .to_string(),
        )
        .unwrap(),
    );
    rows[6] |= (1 << 6) | (1 << 21);
    rows[19] |= 1 | (1 << 9);
    rows[24] |= 1 | (1 << 9);
    engine.item_catalog = crate::human_kill_reward_tests::combat_items_for_ground_test(0.);
    let items = Arc::clone(&engine.item_catalog);
    for actor in engine.active.iter_mut().chain(engine.peers.values_mut()) {
        let loadouts = actor.reservation.character.loadouts.as_mut().unwrap();
        loadouts.inventories[0][12] = Some(3);
        actor.current_inventory_handle = Some(replication::InventoryHandle {
            update_id: 1,
            inventory_index: 0,
        });
        actor.spawned_loadout =
            actor
                .reservation
                .character
                .replication_loadout(&items, 0, actor.wire_profile);
    }
}

pub(crate) fn place_human(engine: &mut Engine<Vec<u8>>, player: u16, position: [f32; 3]) {
    let actor = engine
        .active
        .iter_mut()
        .chain(engine.peers.values_mut())
        .find(|actor| actor.reservation.player == player)
        .unwrap();
    actor.tracking.server_position(
        replication::CharacterPosition {
            x: position[0],
            y: position[1],
            z: position[2],
        },
        player_tracking::PositionOrigin::ServerExit,
        20,
    );
}

/// Bots on an explicit team and squad, at a position and health.
pub(crate) fn add_bots(engine: &mut Engine<Vec<u8>>, bots: &[(u16, [f32; 3], u16)]) -> Vec<u16> {
    let source = crate::bot_registry::identity_tests::populated_engine("Synthetic Barrage");
    let template = source.bot_registry.bots.values().next().unwrap().clone();
    let loadout = engine
        .active
        .iter()
        .chain(engine.peers.values())
        .find_map(|actor| actor.equipped_loadout())
        .unwrap();
    let ids: Vec<u16> = (0..bots.len() as u16).map(|offset| 40 + offset).collect();
    for (&id, &(squad, position, health)) in ids.iter().zip(bots) {
        let mut bot = template.clone();
        bot.target.object_index = id;
        bot.target.loadout = loadout.clone();
        bot.target.squad = replication::SquadAddress::from_index(squad).unwrap();
        bot.health = health;
        bot.damage.health_quarter_points = health;
        let runtime = bot.target.runtime.as_mut().unwrap();
        runtime.position = replication::CharacterPosition {
            x: position[0],
            y: position[1],
            z: position[2],
        };
        runtime.physics_motion = None;
        engine.bot_registry.bots.insert(id, bot);
    }
    ids
}

/// Run world updates on the 50 ms cadence until the bombardment ends.
pub(crate) fn bombard(
    engine: &mut Engine<Vec<u8>>,
    ability: Ability,
    target: [f32; 3],
    start_ms: u64,
) -> u64 {
    engine
        .arm_command_for_test(LEADER, ability, target, ms(start_ms))
        .unwrap();
    let mut now = start_ms;
    while engine.command_effect_active_for_test(ability) {
        now += 50;
        assert!(now < start_ms + 30_000, "bombardment never ended");
        let reply = engine.tick_world(ms(now), now / 1000, format!("tick-{now}"));
        assert!(
            reply.ok,
            "world update at {now} ms failed: {:?}",
            reply.error
        );
    }
    now
}

fn human_health(engine: &Engine<Vec<u8>>, player: u16) -> Option<u16> {
    engine
        .active
        .iter()
        .chain(engine.peers.values())
        .find(|actor| actor.reservation.player == player)
        .and_then(|actor| actor.life.damage_health())
        .map(|(_, health)| health)
}

fn domination_world() -> Engine<Vec<u8>> {
    let mut engine = crate::domination_gameplay_engine::tests::structural_repair::repair_fixture();
    engine.ground_runtime = Some(Default::default());
    let state = &mut engine.domination_gameplay.as_mut().unwrap().state;
    state.ground_respawn_physics = Some(Default::default());
    // The synthetic mission start links its FSP vehicle to an empty spawn,
    // and its ability command names the synthetic platoon token 31.
    Arc::make_mut(&mut state.known_empty_vehicle_spawns).insert(2);
    Arc::make_mut(&mut state.platoons).insert(31, 0);
    let mut rows = state.filter_rows;
    arm_world(&mut engine, &mut rows);
    engine
        .domination_gameplay
        .as_mut()
        .unwrap()
        .state
        .filter_rows = rows;
    place_human(&mut engine, LEADER, [100., 1., 100.]);
    engine.enable_world_ticks();
    engine
}

fn place_structure(engine: &mut Engine<Vec<u8>>, collision_tag: Option<u8>) {
    let state = &mut engine.domination_gameplay.as_mut().unwrap().state;
    state
        .collisions
        .synthetic_structure_roof(7, [104., 1., 100.], 3., collision_tag);
}

/// The live failure: a cluster sample over a placed structure, beside
/// friendly bots (one about to die), the invoker and an enemy.
#[test]
fn domination_cluster_over_structure_friends_invoker_and_enemies_keeps_every_update() {
    let mut engine = domination_world();
    place_structure(&mut engine, Some(9));
    // The leader holds squad 16 on team 1; squads 0.. are the enemy team.
    let bots = add_bots(
        &mut engine,
        &[
            (17, [101., 1., 101.], 8),
            (17, [98., 1., 102.], 400),
            (1, [102., 1., 98.], 400),
            (1, [115., 1., 115.], 400),
        ],
    );
    let leader_before = human_health(&engine, LEADER).unwrap();
    let ended = bombard(&mut engine, Ability::ClusterBomb, [100., 1., 100.], 1_000);
    // The world keeps running after the bombardment, too.
    for now in (ended + 50..=ended + 500).step_by(50) {
        let reply = engine.tick_world(ms(now), now / 1000, "after".into());
        assert!(reply.ok, "{:?}", reply.error);
    }

    let rows = staged_rows(&engine);
    assert!(
        rows.iter()
            .all(|row| row["op"] != "active_ability_effect_failed"),
        "a qualified bombardment must not need the error boundary"
    );
    let hits: Vec<_> = rows
        .iter()
        .filter(|row| row["on_character_hit"] == true && row["damage_source_type"] == 4)
        .collect();
    let attacker = 0x0380_0000 | u32::from(LEADER);
    assert!(hits.iter().all(|hit| hit["attacker"] == attacker));
    let hit = |id: u16| {
        hits.iter()
            .any(|row| row["character_id"] == (0x0380_0000 | u32::from(id)))
    };
    assert!(hit(LEADER), "the invoker stood under the bombardment");
    assert!(hit(bots[0]) && hit(bots[1]), "friendly bots were in range");
    assert!(hit(bots[2]), "the enemy bot was in range");
    assert!(!hit(bots[3]), "the distant enemy was out of range");
    assert!(human_health(&engine, LEADER).is_none_or(|health| health < leader_before));

    // The weak friendly bot died to its own leader's strike: a friendly kill
    // with the command source, and no enemy-kill reward.
    let dead = &engine.bot_registry.bots[&bots[0]];
    assert_eq!(dead.health, 0);
    assert_eq!(dead.damage.last_attacker, attacker);
    let leader = engine.active.as_ref().unwrap();
    assert!(
        leader
            .reward_statistics
            .get(&FRIENDLY_KILLS)
            .copied()
            .unwrap_or(0)
            >= 1
    );
    assert!(engine.bot_registry.bots[&bots[2]].health < 400);
}

/// The error boundary: a bomblet over a structure with no qualified layer
/// is refused, its bombardment ends cleanly, and the world keeps running.
#[test]
fn domination_failed_bomblet_ends_its_bombardment_and_keeps_the_world() {
    let mut engine = domination_world();
    place_structure(&mut engine, None);
    let ended = bombard(&mut engine, Ability::ClusterBomb, [100., 1., 100.], 1_000);
    for now in (ended + 50..=ended + 500).step_by(50) {
        let reply = engine.tick_world(ms(now), now / 1000, "after".into());
        assert!(reply.ok, "{:?}", reply.error);
    }
    let rows = staged_rows(&engine);
    let failures: Vec<_> = rows
        .iter()
        .filter(|row| row["op"] == "active_ability_effect_failed")
        .collect();
    assert_eq!(failures.len(), 1, "{failures:?}");
    let failure = failures[0];
    assert_eq!(failure["error"], "air_strike_sequence_filter_unqualified");
    assert_eq!(failure["ability"], Ability::ClusterBomb.code());
    assert_eq!(failure["invoker"], LEADER);
    // The refused activation is retired, and the next one is admitted.
    assert!(!engine.command_effect_active_for_test(Ability::ClusterBomb));
    let next = bombard(
        &mut engine,
        Ability::ClusterBomb,
        [90., 1., 90.],
        ended + 1_000,
    );
    assert!(next > ended);
}

/// Radial doses on a helicopter the world cannot damage are discarded and
/// recorded; the character damage of the same bombardment still commits.
#[test]
fn domination_refused_helo_doses_do_not_end_the_world() {
    let mut engine = domination_world();
    let state = &mut engine.domination_gameplay.as_mut().unwrap().state;
    state.filter_rows[6] |= 1 << 22;
    state.collisions.synthetic_helo(3, [100., 3., 100.]);
    assert!(state.helo.is_none());
    let ended = bombard(&mut engine, Ability::ClusterBomb, [100., 1., 100.], 1_000);
    let reply = engine.tick_world(ms(ended + 50), 0, "after".into());
    assert!(reply.ok, "{:?}", reply.error);
    let rows = staged_rows(&engine);
    assert!(
        rows.iter()
            .any(|row| row["op"] == "active_ability_helo_damage_failed"
                && row["error"] == "domination_helo_world_required")
    );
    assert!(
        rows.iter()
            .all(|row| row["op"] != "active_ability_effect_failed")
    );
    assert!(
        rows.iter()
            .any(|row| row["on_character_hit"] == true && row["damage_source_type"] == 4)
    );
}

/// A world update that still fails names its cause before the scheduler
/// drops the stream: one process log line and one recorded event.
#[test]
fn failed_world_update_is_recorded_with_its_cause() {
    let mut engine = domination_world();
    let logged = engine.record_world_tick_failure(1, "synthetic_failure", ms(5_000), 7);
    assert_eq!(logged["event"], "world_tick_failed");
    assert_eq!(logged["error"], "synthetic_failure");
    assert_eq!(logged["world_id"], 1);
    assert!(logged.get("record_error").is_none());
    let recorded = events(&engine).pop().unwrap();
    assert_eq!(recorded["op"], "world_tick_failed");
    assert_eq!(recorded["error"], "synthetic_failure");
    assert_eq!(recorded["world_id"], 1);
    assert_eq!(recorded["timestamp_ms"], 5_000);
}

mod staged {
    //! The shared command stage every mode runs, one update at a time, for
    //! each platoon command ability.
    use super::*;
    use crate::{
        active_ability_effects::{
            BlastFalloff, GasBombardmentRules, GuidedArtilleryRules, StrafingRunRules,
        },
        domination_structural_world::{BlastCollision, SequenceCollisionState},
        platoon_effect_engine_tests::{cast, command_engine, native_states},
    };
    use std::collections::BTreeMap;

    type Native = BTreeMap<u32, acquisition_character_effects::NativeState>;
    const CALLER: u32 = 0x0380_0000;

    fn stage(
        engine: &mut Engine<Vec<u8>>,
        native: &Native,
        millis: u64,
        sequences: &SequenceCollisionState,
    ) -> Result<acquisition_character_effects::Candidate, &'static str> {
        let mut rows = [0; 32];
        rows[6] = (1 << 6) | (1 << 21);
        rows[19] = 1 | (1 << 9);
        rows[24] = 1 | (1 << 9);
        let now = ms(millis);
        engine.prepare_acquisition_character_callback(native, now, |engine, native| {
            let result = engine.stage_active_ability_update(
                now,
                native,
                BlastCollision {
                    filter_rows: &rows,
                    limits: Default::default(),
                    sequences,
                    material_costs: &Default::default(),
                    terrain_materials: None,
                },
                None,
            )?;
            Ok((result.audit, result.messages))
        })
    }

    fn with_bots(native: &mut Native, bots: &[u16]) {
        for &bot in bots {
            native.insert(
                CALLER | u32::from(bot),
                acquisition_character_effects::NativeState {
                    invulnerability: Some(0),
                    associated_vehicle: Some(protocol::VictimWasMounted::NoAssociatedVehicle),
                },
            );
        }
    }

    fn falloff(inner: f32, outer: f32) -> BlastFalloff {
        BlastFalloff {
            inner_radius: inner,
            inner_scale: 0.5,
            outer_radius: outer,
            outer_scale: 0.,
        }
    }

    fn structure(collision_tag: Option<u8>) -> SequenceCollisionState {
        let mut sequences = SequenceCollisionState::default();
        sequences.synthetic_structure_roof(7, [103., 1., 100.], 2., collision_tag);
        sequences
    }

    /// Run one activation to completion, committing every update.
    fn run(
        engine: &mut Engine<Vec<u8>>,
        ability: Ability,
        native: &mut Native,
        sequences: &SequenceCollisionState,
    ) -> Vec<Value> {
        cast(engine, ability);
        let mut rows = Vec::new();
        let mut now = 20;
        while engine.command_effect_active_for_test(ability) {
            now += 1;
            assert!(now < 5_000, "{ability:?} never ended");
            let update = stage(engine, native, now, sequences)
                .unwrap_or_else(|error| panic!("{ability:?} update at {now} failed: {error}"));
            rows.extend(update.audit().iter().cloned());
            engine
                .record_acquisition_character_effects(update, native, now)
                .unwrap();
        }
        rows
    }

    #[test]
    fn every_platoon_command_crosses_structure_friends_invoker_enemies_and_ground() {
        let abilities = [
            EffectRules::GuidedArtillery(GuidedArtilleryRules {
                delay_ms: 5,
                damage_delay_ms: 2,
                effective_radius: 8.,
            }),
            EffectRules::GasBombardment(GasBombardmentRules {
                delay_ms: 7,
                damage_delay_ms: 0,
                duration_ms: 300,
                interval_ms: 40,
                num_bomblets: 6,
                bomblet_damage: 90.,
                bomblet: falloff(2., 5.),
                gas_damage_per_second: 50.,
                gas: BlastFalloff {
                    inner_radius: 8.,
                    inner_scale: 0.75,
                    outer_radius: 8.,
                    outer_scale: 1.,
                },
            }),
            EffectRules::StrafingRun(StrafingRunRules {
                delay_ms: 5,
                damage_delay_ms: 1,
                duration_ms: 60,
                interval_ms: 4,
                strafe_length: 30.,
                deviation_radius: 3.,
                bomblet_damage: 120.,
                bomblet: falloff(2., 6.),
            }),
        ];
        for rules in abilities {
            let ability = rules.ability();
            let mut engine = command_engine(rules);
            let bots = add_bots(
                &mut engine,
                &[
                    (17, [101., 3., 101.], 8),
                    (17, [98., 3., 102.], 400),
                    (1, [102., 3., 98.], 400),
                    (1, [117., 3., 117.], 400),
                ],
            );
            let mut native = native_states();
            with_bots(&mut native, &bots);
            let rows = run(&mut engine, ability, &mut native, &structure(Some(9)));
            assert!(
                rows.iter()
                    .all(|row| row["op"] != "active_ability_effect_failed"),
                "{ability:?}: {rows:?}"
            );
            assert!(
                rows.iter()
                    .filter(|row| row["on_character_hit"] == true)
                    .all(|row| row["attacker"] == CALLER),
                "{ability:?}"
            );
            if ability != Ability::GuidedArtillery {
                // The weak friendly bot under the barrage dies to its own
                // leader, credited to the command source.
                let dead = &engine.bot_registry.bots[&bots[0]];
                assert_eq!(dead.health, 0, "{ability:?}");
                assert_eq!(dead.damage.last_attacker, CALLER, "{ability:?}");
                assert!(
                    engine.bot_registry.bots[&bots[2]].health < 400,
                    "{ability:?}"
                );
                assert_eq!(
                    engine.bot_registry.bots[&bots[3]].health, 400,
                    "{ability:?}"
                );
            }
        }
    }

    /// A failure after part of a bomblet's damage was staged discards all of
    /// it: no health, death, statistics, notice or audit row survives, the
    /// activation ends, and the update itself still commits.
    #[test]
    fn failed_bomblet_discards_its_partial_damage_and_ends_the_activation() {
        let mut engine = command_engine(EffectRules::StrafingRun(StrafingRunRules {
            delay_ms: 5,
            damage_delay_ms: 1,
            duration_ms: 60,
            interval_ms: 20,
            strafe_length: 2.,
            deviation_radius: 0.,
            bomblet_damage: 30.,
            bomblet: falloff(2., 6.),
        }));
        // The bot's native protection state is missing, so the bomblet
        // fails on it after both humans' damage is already staged.
        let bots = add_bots(&mut engine, &[(17, [101., 3., 100.], 400)]);
        let mut native = native_states();
        cast(&mut engine, Ability::StrafingRun);
        let sequences = SequenceCollisionState::default();
        let before: Vec<_> = engine
            .peers
            .values()
            .chain(engine.active.iter())
            .map(|actor| {
                (
                    format!("{:?}", actor.life),
                    actor.reward_statistics.clone(),
                    actor.pending_notifications.len(),
                    actor.pending_shared.len(),
                )
            })
            .collect();
        let mut rows = Vec::new();
        for now in 21..=27 {
            let update = stage(&mut engine, &native, now, &sequences).unwrap();
            rows.extend(update.audit().iter().cloned());
            engine
                .record_acquisition_character_effects(update, &mut native, now)
                .unwrap();
        }
        let failures: Vec<_> = rows
            .iter()
            .filter(|row| row["op"] == "active_ability_effect_failed")
            .collect();
        assert_eq!(failures.len(), 1, "{rows:?}");
        assert_eq!(
            failures[0]["error"],
            "domination_structural_character_state_missing"
        );
        assert_eq!(failures[0]["ability"], Ability::StrafingRun.code());
        assert!(rows.iter().all(|row| row["on_character_hit"].is_null()));
        assert!(!engine.command_effect_active_for_test(Ability::StrafingRun));
        let after: Vec<_> = engine
            .peers
            .values()
            .chain(engine.active.iter())
            .map(|actor| {
                (
                    format!("{:?}", actor.life),
                    actor.reward_statistics.clone(),
                    actor.pending_notifications.len(),
                    actor.pending_shared.len(),
                )
            })
            .collect();
        assert_eq!(before, after);
        assert_eq!(engine.bot_registry.bots[&bots[0]].health, 400);
        // The retired activation is published as ended, like a completed one.
        assert!(
            rows.iter()
                .any(|row| row["op"] == "active_ability_update" && row["active_effects"] == 0)
        );
    }

    /// A failing gas pulse is discarded and every command cloud dissipates,
    /// so one bad cloud cannot fail each later world update.
    #[test]
    fn failed_gas_pulse_dissipates_the_cloud_and_keeps_updating() {
        let mut engine = command_engine(EffectRules::GasBombardment(GasBombardmentRules {
            delay_ms: 7,
            damage_delay_ms: 0,
            duration_ms: 1_000,
            interval_ms: 900,
            num_bomblets: 1,
            bomblet_damage: 5.,
            bomblet: falloff(2., 4.),
            gas_damage_per_second: 50.,
            gas: BlastFalloff {
                inner_radius: 9.,
                inner_scale: 0.75,
                outer_radius: 9.,
                outer_scale: 1.,
            },
        }));
        // Inside the cloud, outside the bomblet.
        let bots = add_bots(&mut engine, &[(17, [107., 3., 100.], 400)]);
        let mut native = native_states();
        with_bots(&mut native, &bots);
        cast(&mut engine, Ability::GasBombardment);
        let sequences = SequenceCollisionState::default();
        let mut rows = Vec::new();
        for now in 21..=30 {
            let update = stage(&mut engine, &native, now, &sequences).unwrap();
            rows.extend(update.audit().iter().cloned());
            engine
                .record_acquisition_character_effects(update, &mut native, now)
                .unwrap();
        }
        assert!(
            rows.iter()
                .all(|row| row["op"] != "active_ability_gas_failed")
        );
        // The bot's protection state disappears before the cloud's first pulse.
        native.remove(&(CALLER | u32::from(bots[0])));
        for now in 31..=1_200 {
            let update = stage(&mut engine, &native, now, &sequences)
                .unwrap_or_else(|error| panic!("update at {now} failed: {error}"));
            rows.extend(update.audit().iter().cloned());
            engine
                .record_acquisition_character_effects(update, &mut native, now)
                .unwrap();
        }
        let failures: Vec<_> = rows
            .iter()
            .filter(|row| row["op"] == "active_ability_gas_failed")
            .collect();
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(rows.iter().all(|row| row["op"] != "gas_bombardment_pulse"));
        // No chemical damage from the discarded pulse or the dissipated cloud.
        assert!(rows.iter().all(|row| row["damage_source_type"] != 7));
    }
}
