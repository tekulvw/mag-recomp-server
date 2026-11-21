//! SYNTHETIC queued-damage regression. No game catalogs or account files.
use super::*;
use crate::spawn_progression::{dispatch, ready};

fn reserve(engine: &mut Engine<Vec<u8>>, player: u16) {
    let response = engine.handle(
        Request::Reserve {
            v: BRIDGE_VERSION,
            request_id: format!("synthetic-reserve-{player}"),
            reservation_id: format!("synthetic-reservation-{player:04}"),
            world_id: 1,
            player_id: player,
            account_id: u32::from(player) + 1,
            session_key: "aaaaaaaaaaaaaaaa".into(),
            access_key: "bbbbbbbbbbbbbbbb".into(),
            character: serde_json::from_value(
                json!({"name":format!("Synthetic {player}"),"faction":2,"head":0,
            "voice":0,"universe_id":26,"character_id":7+u32::from(player)}),
            )
            .unwrap(),
        },
        Duration::ZERO,
        1,
    );
    assert!(response.ok, "{:?}", response.error);
}

fn fixture() -> (Engine<Vec<u8>>, u16) {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut engine = super::identity_tests::engine(&format!("Synthetic Grenade {id}"));
    reserve(&mut engine, 0);
    let source = engine.active.as_mut().unwrap();
    ready(source);
    source.life = CharacterLife::Alive {
        generation: 4,
        health_quarter_points: 400,
    };
    source.reservation.character.account_id = Some("synthetic-grenade-owner".into());
    source.reservation.character.character_id = Some(7);
    engine
        .reconcile_bot_population(Duration::ZERO, 1, &mut Default::default())
        .unwrap();
    let bot = engine.bot_combat_snapshots()[0];
    assert_ne!(bot.team, engine.active.as_ref().unwrap().team());
    assert_eq!(engine.bot_registry.bots.len(), 1);
    (engine, bot.object_index)
}
fn queue(engine: &mut Engine<Vec<u8>>, target: u16, generation: u32, attacker: u32) {
    engine
        .active
        .as_mut()
        .unwrap()
        .grenade_damage
        .push_back(PendingDamage {
            target_character_id: 0x0380_0000 | u32::from(target),
            target_reservation_id: None,
            id: "synthetic-grenade-flight-3".into(),
            generation,
            falling: false,
            damage: damage::ResolvedDamage {
                amount_hp: 137.,
                kind: damage::DamageKind::Explosive,
                kinetic_response: false,
                location: 3,
                attacker_id: attacker,
                source_object_id: 0x1080_0000,
                user_data: 0,
            },
        });
}
fn update(engine: &mut Engine<Vec<u8>>, now_ms: u64) -> Result<Vec<OutboundMessage>, &'static str> {
    let sequence = engine.active.as_ref().unwrap().latest_owner_sequence + 1;
    let wire = replication::ObjectUpdate {
        header: replication::UpdateHeader {
            sequence,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 0,
            timestamp_ms: now_ms as u32,
        },
        wire: &[],
    }
    .encode_empty_with_profile(engine.wire_profile)
    .unwrap();
    dispatch(engine, &wire, now_ms)
}

fn firearm_fixture(friendly: bool) -> (Engine<Vec<u8>>, u16) {
    let (mut engine, target) = fixture();
    // Reuse the independent synthetic rifle/armor and admitted equipped snapshot.
    let mut armed = crate::human_kill_reward_tests::fixture();
    let equipped = armed.active.take().unwrap();
    engine.item_catalog = armed.item_catalog;
    let source = engine.active.as_mut().unwrap();
    source.current_inventory_handle = equipped.current_inventory_handle;
    source.spawned_loadout = equipped.spawned_loadout;
    source.observed_primary = equipped.observed_primary;
    source.primary_equipped = true;
    let kit = source.equipped_loadout().unwrap();
    let team = source.team();
    let faction = source.reservation.character.faction;
    let bot = engine.bot_registry.bots.get_mut(&target).unwrap();
    bot.target.squad = replication::SquadAddress::from_index(
        u16::from(if friendly { team } else { 1 - team }) * 16,
    )
    .unwrap();
    bot.target.faction = faction;
    bot.target.loadout = kit;
    bot.health = 120;
    bot.damage.health_quarter_points = 120;
    (engine, target)
}

fn native_firearm_hit(engine: &mut Engine<Vec<u8>>, target: u16) {
    let wire = crate::human_kill_reward_tests::native_hit(0x0380_0000 | u32::from(target), 17);
    dispatch(engine, &wire, 1000).unwrap();
    assert_eq!(engine.bot_registry.bots[&target].health, 0);
    // Friendly classification must not suppress the accepted native death notice.
    let source = engine.active.as_ref().unwrap();
    assert_eq!(source.pending_notifications.len(), 1);
    assert_eq!(
        engine.bot_registry.bots[&target].damage.last_attacker,
        0x0380_0000 | u32::from(source.reservation.player)
    );
}

#[test]
fn bot_firearm_kd_counts_registered_bot_death_once() {
    let (mut engine, target) = firearm_fixture(false);
    native_firearm_hit(&mut engine, target);
    let totals = crate::bot_firearm_kd::Totals {
        kills: 0,
        assists: 0,
        deaths: 1,
    };
    assert_eq!(engine.bot_registry.bots[&target].firearm_kd, totals);
    let wire = crate::human_kill_reward_tests::native_hit(0x0380_0000 | u32::from(target), 17);
    assert!(dispatch(&mut engine, &wire, 1001).unwrap().is_empty());
    assert_eq!(engine.bot_registry.bots[&target].firearm_kd, totals);
}

#[test]
fn bot_firearm_kd_human_hit_keeps_totals_on_refusal_and_nonlethal_damage() {
    let (mut engine, target) = firearm_fixture(false);
    let hit = crate::human_kill_reward_tests::native_hit(0x0380_0000 | u32::from(target), 17);
    engine.max_log = engine.written;
    assert_eq!(
        dispatch(&mut engine, &hit, 1000).unwrap_err(),
        "log_limit_reached"
    );
    assert_eq!(engine.bot_registry.bots[&target].health, 120);
    assert_eq!(
        engine.bot_registry.bots[&target].firearm_kd,
        Default::default()
    );
    assert!(engine.pending_reward_batch().is_empty());

    let (mut engine, target) = firearm_fixture(false);
    let hit = crate::human_kill_reward_tests::native_hit(0x0380_0000 | u32::from(target), 17);
    let bot = engine.bot_registry.bots.get_mut(&target).unwrap();
    bot.health = 400;
    bot.damage.health_quarter_points = 400;
    dispatch(&mut engine, &hit, 1001).unwrap();
    assert_eq!(engine.bot_registry.bots[&target].health, 240);
    assert_eq!(
        engine.bot_registry.bots[&target].firearm_kd,
        Default::default()
    );
}

#[test]
fn bot_firearm_kd_survives_respawn_but_not_a_new_allocation() {
    let (mut engine, target) = firearm_fixture(false);
    let initial = &engine.bot_registry.bots[&target];
    let old_life = crate::bot_firearm_kd::Life {
        index: target,
        generation: initial.target.runtime.unwrap().generation,
        team: initial.target.squad.team(),
        health: initial.health,
    };
    let allocation = initial.allocation_generation;
    let staged = crate::bot_firearm_kd::Plan::prepare(&engine, 1, None, Some(old_life)).unwrap();
    assert_eq!(
        staged.install(&mut engine).err(),
        Some("stale_bot_firearm_kd_plan")
    );

    native_firearm_hit(&mut engine, target);
    engine.team_waves[usize::from(old_life.team)] = Some(TeamWaveClock {
        first_deadline: Duration::from_millis(1050),
        period: Duration::from_secs(10),
    });
    engine.tick_bots(Duration::from_millis(1100), 1).unwrap();
    let respawned = &engine.bot_registry.bots[&target];
    assert_eq!(respawned.health, 400);
    assert_eq!(
        respawned.target.runtime.unwrap().generation,
        old_life.generation + 1
    );
    assert_eq!(respawned.allocation_generation, allocation);
    assert_eq!(respawned.firearm_kd.deaths, 1);
    assert_eq!(
        crate::bot_firearm_kd::Plan::prepare(
            &engine,
            1,
            None,
            Some(crate::bot_firearm_kd::Life {
                health: respawned.health,
                ..old_life
            })
        )
        .err(),
        Some("stale_bot_firearm_kd_life")
    );

    engine
        .retire_bot_for_human(target, Duration::from_millis(1200), 1)
        .unwrap();
    let bootstraps = super::identity_tests::engine("Synthetic Bot Reallocation");
    engine.item_catalog = bootstraps.item_catalog;
    engine
        .reconcile_bot_population(Duration::from_millis(1200), 1, &mut Default::default())
        .unwrap();
    let replacement = &engine.bot_registry.bots[&target];
    assert!(replacement.allocation_generation > allocation);
    assert_eq!(replacement.firearm_kd, Default::default());
    engine.clear_bot_world();
    assert!(engine.bot_registry.bots.is_empty());
}

fn scheduler_fixture() -> (Engine<Vec<u8>>, u16) {
    let (mut engine, shooter) = firearm_fixture(false);
    engine.item_catalog = crate::human_kill_reward_tests::bot_firearm_items();
    engine.match_started = true;
    engine.bot_registry.config.as_mut().unwrap().firearms = crate::bot_firearms::Policy {
        reaction_ms: 1,
        turn_degrees_per_second: 720.,
        aim_tolerance_degrees: 15.,
        accuracy_per_mille: 1000,
        far_accuracy_per_mille: 1000,
        acquisition_fov_degrees: 360.,
        ..Default::default()
    };
    let actor = engine.active.as_mut().unwrap();
    actor.life = CharacterLife::Alive {
        generation: 4,
        health_quarter_points: 120,
    };
    actor.owner_fields.state = Some(replication::CharacterState {
        state: 1,
        stance: 0,
        user_data: [0; 3],
    });
    actor.tracking.server_position(
        replication::CharacterPosition {
            x: 2.,
            y: 0.,
            z: 6.,
        },
        player_tracking::PositionOrigin::ServerSpawn,
        0,
    );
    let runtime = engine.bot_registry.bots[&shooter].target.runtime.unwrap();
    let world =
        bot_physics::WorldTerrain::new(engine.collision_world.as_ref(), engine.map_name()).unwrap();
    let motion = world
        .advance(
            runtime.physics_motion.unwrap(),
            [0.; 3],
            0.05,
            Default::default(),
        )
        .unwrap();
    engine
        .bot_registry
        .bots
        .get_mut(&shooter)
        .unwrap()
        .target
        .runtime
        .as_mut()
        .unwrap()
        .physics_motion = Some(motion);
    assert!(crate::bot_firearms::safe(
        &engine.bot_registry.bots[&shooter]
    ));
    (engine, shooter)
}

fn dry_resupply_fixture() -> (Engine<Vec<u8>>, u16) {
    let (mut engine, shooter) = scheduler_fixture();
    engine.active.as_mut().unwrap().life = CharacterLife::AwaitingRespawn { generation: 4 };
    engine.tick_bot_firearms(Duration::ZERO, 1).unwrap();
    let state = &mut engine.bot_registry.bots.get_mut(&shooter).unwrap().firearm;
    assert_eq!((state.magazine, state.reserve), (9, 9));
    state.magazine = 0;
    state.reserve = 0;
    (engine, shooter)
}

fn enable_synthetic_resupply(engine: &mut Engine<Vec<u8>>, shooter: u16) {
    let origin = engine.bot_registry.bots[&shooter]
        .target
        .runtime
        .unwrap()
        .physical_position();
    engine.install_synthetic_bot_resupply(bot_resupply_catalog::Catalog::synthetic(origin, 0.375));
}

#[test]
fn bot_resupply_dry_idle_waits_then_refills_reserve_and_normally_fires() {
    let (mut engine, shooter) = dry_resupply_fixture();
    enable_synthetic_resupply(&mut engine, shooter);
    let tick = |engine: &mut Engine<Vec<u8>>, ms| {
        engine
            .tick_bot_firearms(Duration::from_millis(ms), 1)
            .unwrap();
    };
    tick(&mut engine, 1000);
    let started = engine.bot_registry.bots[&shooter]
        .resupply
        .expect("AI Start");
    tick(&mut engine, 1374);
    assert_eq!(engine.bot_registry.bots[&shooter].resupply, Some(started));
    let state = &engine.bot_registry.bots[&shooter].firearm;
    assert_eq!((state.magazine, state.reserve), (0, 0));
    tick(&mut engine, 1375);
    assert!(engine.bot_registry.bots[&shooter].resupply.is_none());
    let state = &engine.bot_registry.bots[&shooter].firearm;
    assert_eq!((state.magazine, state.reserve), (0, 9));
    tick(&mut engine, 1400);
    let bot = &engine.bot_registry.bots[&shooter];
    assert!(bot.firearm.item(bot.target.loadout.primary).reloading);
    assert_eq!((bot.firearm.magazine, bot.firearm.reserve), (0, 9));
    tick(&mut engine, 2400);
    let bot = &engine.bot_registry.bots[&shooter];
    assert!(!bot.firearm.item(bot.target.loadout.primary).reloading);
    assert_eq!((bot.firearm.magazine, bot.firearm.reserve), (9, 0));
    engine.active.as_mut().unwrap().life = CharacterLife::Alive {
        generation: 4,
        health_quarter_points: 120,
    };
    for ms in (2450..3300).step_by(50) {
        tick(&mut engine, ms);
    }
    assert!(!engine.active.as_ref().unwrap().life.is_alive());
    assert_eq!(engine.bot_registry.bots[&shooter].firearm.magazine, 8);
}

#[test]
fn bot_resupply_pauses_fixed_move_and_cancels_for_displacement_inside_box() {
    let (mut engine, shooter) = dry_resupply_fixture();
    enable_synthetic_resupply(&mut engine, shooter);
    engine.tick_bot_firearms(Duration::from_secs(1), 1).unwrap();
    let slot = engine.bot_registry.bots[&shooter].slot;
    let held = engine.bot_registry.bots[&shooter].resupply;
    assert!(held.is_some());
    engine.bot_registry.config.as_mut().unwrap().slots[slot].goal =
        Some(replication::CharacterPosition {
            x: 4.,
            y: 0.,
            z: 2.,
        });
    engine.tick_bots(Duration::from_millis(1200), 1).unwrap();
    let bot = engine.bot_registry.bots.get_mut(&shooter).unwrap();
    assert!(!bot.move_attempted);
    assert!(matches!(
        bot.target.runtime.unwrap().phase,
        development_target::BotPhase::Complete
    ));
    assert_eq!(bot.resupply, held);
    bot.target
        .runtime
        .as_mut()
        .unwrap()
        .physics_motion
        .as_mut()
        .unwrap()
        .position[0] += 0.25;
    engine
        .tick_bot_firearms(Duration::from_millis(1375), 1)
        .unwrap();
    let bot = &engine.bot_registry.bots[&shooter];
    assert!(bot.resupply.is_none());
    assert_eq!((bot.firearm.magazine, bot.firearm.reserve), (0, 0));
    assert!(String::from_utf8_lossy(&engine.writer).contains("moved_during_service"));
}

#[test]
fn bot_resupply_ordinary_opt_out_and_refused_pulse_preserve_ammunition() {
    let (mut disabled, shooter) = dry_resupply_fixture();
    disabled
        .tick_bot_firearms(Duration::from_secs(1), 1)
        .unwrap();
    disabled
        .tick_bot_firearms(Duration::from_secs(2), 1)
        .unwrap();
    assert!(disabled.bot_registry.bots[&shooter].resupply.is_none());
    assert_eq!(disabled.bot_registry.bots[&shooter].firearm.reserve, 0);

    let (mut engine, shooter) = dry_resupply_fixture();
    enable_synthetic_resupply(&mut engine, shooter);
    engine.tick_bot_firearms(Duration::from_secs(1), 1).unwrap();
    let before = engine.bot_registry.bots[&shooter].resupply.unwrap();
    let queued = engine.active.as_ref().unwrap().pending_shared.clone();
    engine.max_log = engine.written;
    assert_eq!(
        engine
            .tick_bot_firearms(Duration::from_millis(1375), 1)
            .unwrap_err(),
        "log_limit_reached"
    );
    let bot = &engine.bot_registry.bots[&shooter];
    assert_eq!(bot.resupply, Some(before));
    assert_eq!((bot.firearm.magazine, bot.firearm.reserve), (0, 0));
    assert_eq!(engine.active.as_ref().unwrap().pending_shared, queued);
}

#[test]
fn bot_resupply_cancels_after_recorded_damage_even_if_healed_to_exact_prior_health() {
    let (mut engine, shooter) = dry_resupply_fixture();
    enable_synthetic_resupply(&mut engine, shooter);
    let started = Duration::from_secs(1);
    engine.tick_bot_firearms(started, 1).unwrap();
    let bot = &engine.bot_registry.bots[&shooter];
    assert!(bot.resupply.is_some());
    let generation = bot.target.runtime.unwrap().generation;
    let prior = bot.health;
    let healed = replication::CharacterDamage {
        health_quarter_points: prior,
        ..bot.damage
    };
    let hurt = replication::CharacterDamage {
        health_quarter_points: prior - 10,
        ..healed
    };
    engine
        .record(json!({"op":"synthetic_accepted_damage"}), 1)
        .unwrap();
    engine
        .commit_bot_damage(shooter, generation, hurt, Duration::from_millis(1100))
        .unwrap();
    engine
        .record(json!({"op":"synthetic_accepted_heal"}), 1)
        .unwrap();
    engine
        .commit_bot_healing(shooter, generation, prior - 10, healed, 400)
        .unwrap();
    assert_eq!(engine.bot_registry.bots[&shooter].health, prior);
    engine
        .tick_bot_firearms(Duration::from_millis(1375), 1)
        .unwrap();
    let bot = &engine.bot_registry.bots[&shooter];
    assert!(bot.resupply.is_none());
    assert_eq!((bot.firearm.magazine, bot.firearm.reserve), (0, 0));
    assert!(String::from_utf8_lossy(&engine.writer).contains("combat_interrupt"));
}

#[test]
fn bot_resupply_refused_equipment_cancel_keeps_old_ammo_binding_stable() {
    let (mut engine, shooter) = dry_resupply_fixture();
    enable_synthetic_resupply(&mut engine, shooter);
    engine.tick_bot_firearms(Duration::from_secs(1), 1).unwrap();
    let bot = engine.bot_registry.bots.get_mut(&shooter).unwrap();
    let session = bot.resupply.unwrap();
    bot.target.loadout.handle.update_id += 1;
    engine.max_log = engine.written;
    assert_eq!(
        engine
            .tick_bot_firearms(Duration::from_millis(1375), 1)
            .unwrap_err(),
        "log_limit_reached"
    );
    let bot = &engine.bot_registry.bots[&shooter];
    assert_eq!(bot.resupply, Some(session));
    assert_eq!((bot.firearm.magazine, bot.firearm.reserve), (0, 0));
}

#[test]
fn bot_firearm_kd_scheduler_counts_incapacitating_hit_once() {
    let (engine, shooter) = scheduler_fixture();
    let mut engine = engine
        .with_incapacitation_rules(incapacitation::Rules {
            schema_version: 1,
            incapacitated_damage_threshold_hp: 83.,
            incapacitated_duration_seconds: 7.,
            excluded_character_states: vec![22],
        })
        .unwrap();
    let mut death_rows = Vec::new();
    for ms in (50..500).step_by(50) {
        death_rows.extend(
            engine
                .tick_bot_firearms(Duration::from_millis(ms), 1)
                .unwrap(),
        );
    }
    let victim = engine.active.as_ref().unwrap();
    assert!(
        matches!(
            victim.life,
            CharacterLife::Incapacitated { generation: 4, .. }
        ),
        "{}",
        String::from_utf8_lossy(&engine.writer)
    );
    assert_eq!(victim.reward_statistics.get(&12), Some(&1));
    assert!(!victim.reward_statistics.contains_key(&10));
    assert_eq!(
        engine.bot_registry.bots[&shooter].firearm_kd,
        crate::bot_firearm_kd::Totals {
            kills: 1,
            assists: 0,
            deaths: 0
        }
    );
    assert_eq!(death_rows.len(), 1);
    assert_eq!(death_rows[0].payload_hex, "02040000030a000b000c01");
    assert!(engine.pending_reward_batch().is_empty());
    engine
        .tick_incapacitation(Duration::from_secs(10), 1)
        .unwrap();
    assert!(matches!(
        engine.active.as_ref().unwrap().life,
        CharacterLife::AwaitingRespawn { generation: 4 }
    ));
    assert_eq!(
        engine.active.as_ref().unwrap().reward_statistics.get(&12),
        Some(&1)
    );
    assert_eq!(engine.bot_registry.bots[&shooter].firearm_kd.kills, 1);
}

fn scheduler_bot_opponents() -> (Engine<Vec<u8>>, u16, u16) {
    let (mut engine, shooter) = scheduler_fixture();
    let human_squad = engine.spawn_catalog.as_ref().unwrap().for_squad(0).unwrap();
    let observer = engine.active.as_mut().unwrap();
    observer.life = CharacterLife::AwaitingRespawn { generation: 4 };
    observer.squad_catalog = std::sync::Arc::new(human_squad);
    engine.maximum_players = 4;
    engine.bot_registry.total = 3;
    let items = engine.item_catalog.clone();
    // Reconcile using the bootstrap catalog; the scheduler uses the fixture rifle below.
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let bootstraps = super::identity_tests::engine(&format!("Synthetic Bot Opponent {id}"));
    engine.item_catalog = bootstraps.item_catalog;
    engine
        .reconcile_bot_population(Duration::ZERO, 1, &mut Default::default())
        .unwrap();
    engine.item_catalog = items;
    let target = *engine
        .bot_registry
        .bots
        .keys()
        .find(|&&index| index != shooter)
        .unwrap();
    let kit = engine.bot_registry.bots[&shooter].target.loadout.clone();
    let target_bot = engine.bot_registry.bots.get_mut(&target).unwrap();
    target_bot.target.loadout = kit;
    target_bot.health = 120;
    target_bot.damage.health_quarter_points = 120;
    target_bot.task.recovering = true;
    assert_ne!(
        target_bot.target.squad.team(),
        engine.bot_registry.bots[&shooter].target.squad.team()
    );
    (engine, shooter, target)
}

#[test]
fn bot_resupply_later_ordinary_shot_interrupts_fresh_due_observation() {
    let (mut engine, opponent, servicing) = scheduler_bot_opponents();
    let physical = engine.bot_registry.bots[&servicing]
        .target
        .runtime
        .unwrap()
        .physics_motion
        .unwrap();
    let terrain =
        bot_physics::WorldTerrain::new(engine.collision_world.as_ref(), engine.map_name()).unwrap();
    let grounded = terrain
        .advance(physical, [0.; 3], 0.05, Default::default())
        .unwrap();
    assert!(grounded.grounded);
    let enemy = engine.bot_registry.bots[&opponent]
        .target
        .runtime
        .unwrap()
        .physical_position();
    let direct = (grounded.position[0] - enemy[0]).atan2(grounded.position[2] - enemy[2]);
    let yaw = (direct.rem_euclid(std::f64::consts::TAU) * 1024. / std::f64::consts::TAU) as u16;
    let adversary = engine.bot_registry.bots.get_mut(&opponent).unwrap();
    adversary.yaw = yaw;
    adversary.task.recovering = true;
    let recipient = engine.bot_registry.bots.get_mut(&servicing).unwrap();
    recipient.yaw = yaw; // Looks away from the opponent.
    recipient.task.recovering = false;
    recipient.target.runtime.as_mut().unwrap().physics_motion = Some(grounded);
    recipient.health = 400;
    recipient.damage.health_quarter_points = 400;
    engine
        .bot_registry
        .config
        .as_mut()
        .unwrap()
        .firearms
        .acquisition_fov_degrees = 60.;
    let tick = |engine: &mut Engine<Vec<u8>>, ms| {
        engine
            .tick_bot_firearms(Duration::from_millis(ms), 1)
            .unwrap();
    };
    tick(&mut engine, 0);
    let state = &mut engine
        .bot_registry
        .bots
        .get_mut(&servicing)
        .unwrap()
        .firearm;
    assert_eq!((state.magazine, state.reserve), (9, 9));
    state.magazine = 0;
    state.reserve = 0;
    enable_synthetic_resupply(&mut engine, servicing);
    tick(&mut engine, 1000);
    assert!(engine.bot_registry.bots[&servicing].resupply.is_some());
    engine
        .bot_registry
        .bots
        .get_mut(&opponent)
        .unwrap()
        .task
        .recovering = false;
    tick(&mut engine, 1250); // Acquire; reaction delay prevents a same-sample shot.
    assert_eq!(engine.bot_registry.bots[&servicing].health, 400);
    engine.bot_registry.firearm_cursor = servicing; // Recipient observes quiet first.
    tick(&mut engine, 1375);
    let recipient = &engine.bot_registry.bots[&servicing];
    assert_eq!(
        recipient.health, 240,
        "ordinary later shooter must land damage"
    );
    assert_eq!(
        (recipient.firearm.magazine, recipient.firearm.reserve),
        (0, 0)
    );
    assert!(recipient.resupply.is_none());
    let log = String::from_utf8_lossy(&engine.writer);
    assert!(log.contains("combat_interrupt"));
    assert!(!log.contains("bot_resupply_pulse"));
}

fn firearm_assist_fixture(timeout_ms: u32) -> (Engine<Vec<u8>>, u16, u16) {
    let (mut engine, shooter, target) = scheduler_bot_opponents();
    engine = engine
        .with_firearm_assist_rules(crate::firearm_assists::Rules {
            schema_version: 1,
            application_id: 20231,
            assist_timeout_ms: timeout_ms,
        })
        .unwrap();
    let ally_team = engine.bot_registry.bots[&shooter].target.squad.team();
    let squad = engine
        .spawn_catalog
        .as_ref()
        .unwrap()
        .for_squad(u16::from(ally_team) * 16)
        .unwrap();
    let human = engine.active.as_mut().unwrap();
    human.squad_catalog = std::sync::Arc::new(squad);
    human.life = CharacterLife::Alive {
        generation: 4,
        health_quarter_points: 400,
    };
    (engine, shooter, target)
}

fn zero_damage_firearm_hit(target: u16) -> Vec<u8> {
    let mut hit = crate::human_kill_reward_tests::native_hit(0x03800000 | u32::from(target), 77);
    // Existing 020f fixture geometry is valid for the ordinary profile, but the
    // zero penetration field produces a zero HP hit before the bot kills it.
    hit[37..39].copy_from_slice(&[0, 0]);
    hit
}

#[test]
fn firearm_assists_human_zero_damage_then_bot_terminal_counts_once() {
    let (mut engine, shooter, target) = firearm_assist_fixture(901);
    let hit = zero_damage_firearm_hit(target);
    dispatch(&mut engine, &hit, 1).unwrap();
    assert_eq!(engine.bot_registry.bots[&target].health, 120);
    let mut messages = Vec::new();
    for ms in (50..900).step_by(50) {
        messages.extend(
            engine
                .tick_bot_firearms(Duration::from_millis(ms), 1)
                .unwrap(),
        );
    }
    assert_eq!(engine.bot_registry.bots[&target].health, 0);
    assert_eq!(engine.bot_registry.bots[&shooter].firearm_kd.kills, 1);
    assert_eq!(
        engine.active.as_ref().unwrap().reward_statistics.get(&11),
        Some(&1)
    );
    let messages = engine.drain_shared(messages, 1).unwrap();
    assert_eq!(
        messages
            .iter()
            .filter(|message| message.payload_hex == "02040000010b01")
            .count(),
        1
    );
    assert!(engine.pending_reward_batch().is_empty());
}

fn assist_bot(engine: &Engine<Vec<u8>>, index: u16) -> firearm_assists::Fighter<'_> {
    firearm_assists::Fighter {
        index,
        reservation: None,
        generation: engine.bot_registry.bots[&index]
            .target
            .runtime
            .unwrap()
            .generation,
    }
}

fn preview_bot_terminal(
    engine: &Engine<Vec<u8>>,
    killer: u16,
    victim: u16,
    now: Duration,
) -> firearm_assists::Plan {
    firearm_assists::Plan::prepare(
        engine,
        1,
        now,
        assist_bot(engine, killer),
        assist_bot(engine, victim),
        true,
    )
    .unwrap()
}

#[test]
fn firearm_assists_zero_retains_same_millisecond_and_expires_next_millisecond() {
    let (mut engine, shooter, target) = firearm_assist_fixture(0);
    dispatch(&mut engine, &zero_damage_firearm_hit(target), 1).unwrap();
    assert_eq!(
        preview_bot_terminal(&engine, shooter, target, Duration::from_micros(1999))
            .audit()
            .len(),
        1
    );
    assert!(
        preview_bot_terminal(&engine, shooter, target, Duration::from_millis(2))
            .audit()
            .is_empty()
    );
}

#[test]
fn firearm_assists_ignore_refused_reports_and_survive_only_the_same_attacker_allocation() {
    let (mut refused, shooter, target) = firearm_assist_fixture(50);
    let hit = zero_damage_firearm_hit(target);
    refused.max_log = refused.written;
    assert_eq!(
        dispatch(&mut refused, &hit, 1).unwrap_err(),
        "log_limit_reached"
    );
    assert!(
        preview_bot_terminal(&refused, shooter, target, Duration::from_millis(2))
            .audit()
            .is_empty()
    );

    let (mut engine, shooter, target) = firearm_assist_fixture(50);
    dispatch(&mut engine, &zero_damage_firearm_hit(target), 1).unwrap();
    let source = engine.active.as_mut().unwrap();
    source.life = CharacterLife::Alive {
        generation: 5,
        health_quarter_points: 400,
    };
    let now = Duration::from_millis(51);
    assert_eq!(
        preview_bot_terminal(&engine, shooter, target, now)
            .audit()
            .len(),
        1
    );
    // A fresh recorded admission can reuse the same string after the old
    // fixed-world release tombstone has aged out. Its allocation is distinct.
    engine.active.as_mut().unwrap().allocation_sequence += 1;
    assert!(
        preview_bot_terminal(&engine, shooter, target, now)
            .audit()
            .is_empty()
    );
}

#[test]
fn firearm_assists_reject_stale_detached_plans_after_revision_round_or_victim_life_changes() {
    let (mut engine, shooter, target) = firearm_assist_fixture(50);
    let now = Duration::from_millis(1);
    let prepare = |engine: &Engine<Vec<u8>>| {
        firearm_assists::Plan::prepare(
            engine,
            1,
            now,
            firearm_assists::Fighter::human(engine.active.as_ref().unwrap()),
            assist_bot(engine, target),
            false,
        )
        .unwrap()
    };
    let stale = prepare(&engine);
    dispatch(&mut engine, &zero_damage_firearm_hit(target), 1).unwrap();
    assert_eq!(stale.install(&mut engine), Err("stale_firearm_assist_plan"));
    assert_eq!(
        preview_bot_terminal(&engine, shooter, target, now)
            .audit()
            .len(),
        1
    );

    let stale = prepare(&engine);
    engine.round_number += 1;
    assert_eq!(stale.install(&mut engine), Err("stale_firearm_assist_plan"));
    let stale = prepare(&engine);
    let bot = engine.bot_registry.bots.get_mut(&target).unwrap();
    bot.target.runtime.as_mut().unwrap().generation += 1;
    assert_eq!(stale.install(&mut engine), Err("stale_firearm_assist_plan"));
}

fn bot_first_assist_hit(engine: &mut Engine<Vec<u8>>, target: u16) -> u64 {
    let bot = engine.bot_registry.bots.get_mut(&target).unwrap();
    bot.health = 240;
    bot.damage.health_quarter_points = 240;
    for ms in (50..900).step_by(50) {
        engine
            .tick_bot_firearms(Duration::from_millis(ms), 1)
            .unwrap();
        if engine.bot_registry.bots[&target].health == 80 {
            return ms;
        }
    }
    panic!("the allied bot did not land its admitted shot");
}

#[test]
fn firearm_assists_bot_hit_then_human_terminal_counts_bot() {
    let (mut engine, shooter, target) = firearm_assist_fixture(901);
    let ms = bot_first_assist_hit(&mut engine, target);
    let wire = crate::human_kill_reward_tests::native_hit(0x03800000 | u32::from(target), 77);
    dispatch(&mut engine, &wire, ms + 1).unwrap();
    assert_eq!(engine.bot_registry.bots[&target].health, 0);
    assert_eq!(engine.bot_registry.bots[&shooter].firearm_kd.assists, 1);
    assert_eq!(engine.bot_registry.bots[&shooter].firearm_kd.kills, 0);
    dispatch(&mut engine, &wire, ms + 2).unwrap();
    assert_eq!(engine.bot_registry.bots[&shooter].firearm_kd.assists, 1);
}

#[test]
fn firearm_assists_stale_terminal_cannot_overwrite_newer_bot_totals() {
    let (mut engine, shooter, target) = firearm_assist_fixture(901);
    let now = Duration::from_millis(bot_first_assist_hit(&mut engine, target) + 1);
    let terminal = firearm_assists::Plan::prepare(
        &engine,
        1,
        now,
        firearm_assists::Fighter::human(engine.active.as_ref().unwrap()),
        assist_bot(&engine, target),
        true,
    )
    .unwrap();
    assert_eq!(terminal.audit().len(), 1);
    terminal.check_before_record(&engine).unwrap();
    let victim = &engine.bot_registry.bots[&target];
    let generation = victim.target.runtime.unwrap().generation;
    let damage = replication::CharacterDamage {
        health_quarter_points: 0,
        ..victim.damage
    };
    engine
        .commit_bot_damage(target, generation, damage, now)
        .unwrap();
    engine
        .bot_registry
        .bots
        .get_mut(&shooter)
        .unwrap()
        .firearm_kd
        .assists = 3;
    assert_eq!(
        terminal.install(&mut engine),
        Err("stale_firearm_assist_plan")
    );
    assert_eq!(engine.bot_registry.bots[&shooter].firearm_kd.assists, 3);
}

#[test]
fn firearm_assists_four_contributors_reach_seven_recipients_within_shared_budget() {
    let (engine, shooter) = scheduler_fixture();
    let mut engine = engine
        .with_firearm_assist_rules(firearm_assists::Rules {
            schema_version: 1,
            application_id: 20231,
            assist_timeout_ms: 123,
        })
        .unwrap();
    engine.maximum_players = 8;
    let admitted_shooter = engine.bot_registry.bots[&shooter].clone();
    let team = admitted_shooter.target.squad.team();
    let squad = std::sync::Arc::new(
        engine
            .spawn_catalog
            .as_ref()
            .unwrap()
            .for_squad(u16::from(team) * 16)
            .unwrap(),
    );
    for player in 2..8 {
        reserve(&mut engine, player);
        let actor = engine.active.as_mut().unwrap();
        ready(actor);
        actor.squad_catalog = squad.clone();
        actor.life = CharacterLife::Alive {
            generation: 1,
            health_quarter_points: 400,
        };
    }
    // The two-player scheduler fixture shrinks its bot population during the
    // earlier reservations. Retain its actual fighter as the eighth participant.
    engine.bot_registry.total = 8;
    engine.bot_registry.bots.insert(shooter, admitted_shooter);
    let victim_id = "synthetic-reservation-0000";
    let now = Duration::from_millis(1);
    for player in 2..6 {
        let contributor = &engine.peers[&format!("synthetic-reservation-{player:04}")];
        let assist = firearm_assists::Plan::prepare(
            &engine,
            1,
            now,
            firearm_assists::Fighter::human(contributor),
            firearm_assists::Fighter::human(&engine.peers[victim_id]),
            false,
        )
        .unwrap();
        assist.install(&mut engine).unwrap();
    }
    let victim = &engine.peers[victim_id];
    let terminal = firearm_assists::Plan::prepare(
        &engine,
        1,
        now,
        assist_bot(&engine, shooter),
        firearm_assists::Fighter::human(victim),
        true,
    )
    .unwrap();
    assert_eq!(terminal.audit().len(), 4);
    let stats = events::plan_bot_firearm_assists(&engine, &terminal).unwrap();
    let death = events::plan_bot_killed_human(&engine, victim, 4).unwrap();
    terminal.check_before_record(&engine).unwrap();
    engine
        .record(
            json!({"op":"synthetic_bot_firearm_terminal","firearm_assists":terminal.audit()}),
            1,
        )
        .unwrap();
    let victim = engine.peers.get_mut(victim_id).unwrap();
    let model = replication::CharacterDamage {
        health_quarter_points: 0,
        ..victim.last_damage
    };
    victim.apply_peer_damage(4, model, now).unwrap();
    terminal.install(&mut engine).unwrap();
    let mut direct = death.install(&mut engine).unwrap();
    direct.extend(stats.commit(&mut engine));
    let mut messages = engine.drain_shared(direct, 1).unwrap();
    assert!(messages.len() <= 32);
    let rest = engine.drain_shared(Vec::new(), 1).unwrap();
    assert!(rest.len() <= 32);
    messages.extend(rest);
    for contributor in 2..6 {
        let row = format!("0204{contributor:04x}010b01");
        let recipients: std::collections::BTreeSet<_> = messages
            .iter()
            .filter(|message| message.payload_hex == row)
            .map(|message| message.player_id)
            .collect();
        assert_eq!(recipients, [0, 2, 3, 4, 5, 6, 7].into_iter().collect());
    }
}

#[test]
fn bot_firearm_kd_scheduler_updates_both_bot_allocations_after_record() {
    let (mut engine, shooter, target) = scheduler_bot_opponents();
    let fired_at = (50..900).step_by(50).find(|&ms| {
        engine
            .tick_bot_firearms(Duration::from_millis(ms), 1)
            .unwrap();
        engine.bot_registry.bots[&target].health == 0
    });
    assert_eq!(
        engine.bot_registry.bots[&shooter].firearm_kd.kills,
        1,
        "{}",
        String::from_utf8_lossy(&engine.writer)
    );
    assert_eq!(engine.bot_registry.bots[&target].firearm_kd.deaths, 1);
    assert!(engine.active.as_ref().unwrap().reward_statistics.is_empty());
    assert!(engine.pending_reward_batch().is_empty());

    let (mut engine, shooter, target) = scheduler_bot_opponents();
    let fired_at = fired_at.unwrap();
    for ms in (50..fired_at).step_by(50) {
        engine
            .tick_bot_firearms(Duration::from_millis(ms), 1)
            .unwrap();
    }
    let before_queues = engine.active.as_ref().unwrap().pending_shared.clone();
    let before_ammo = engine.bot_registry.bots[&shooter].firearm.magazine;
    engine.max_log = engine.written;
    assert_eq!(
        engine
            .tick_bot_firearms(Duration::from_millis(fired_at), 1)
            .unwrap_err(),
        "log_limit_reached"
    );
    assert_eq!(engine.bot_registry.bots[&target].health, 120);
    assert_eq!(
        engine.bot_registry.bots[&target].firearm_kd,
        Default::default()
    );
    assert_eq!(
        engine.bot_registry.bots[&shooter].firearm_kd,
        Default::default()
    );
    assert_eq!(
        engine.bot_registry.bots[&shooter].firearm.magazine,
        before_ammo
    );
    assert_eq!(
        engine.active.as_ref().unwrap().pending_shared,
        before_queues
    );
}

#[test]
fn friendly_native_hit_records_bot_friendly_kill_without_enemy_xp() {
    let (mut engine, target) = firearm_fixture(true);
    assert_eq!(
        engine.bot_combat_snapshots()[0].team,
        engine.active.as_ref().unwrap().team()
    );
    native_firearm_hit(&mut engine, target);
    let source = engine.active.as_ref().unwrap();
    assert_eq!(source.reward_statistics.get(&14), Some(&1));
    assert_eq!(source.reward_statistics.get(&10), None);
    assert!(engine.pending_reward_batch().is_empty());
}

#[test]
fn native_hit_relation_base_applies_to_registry_bots_without_faction_inference() {
    for friendly in [true, false] {
        let (mut engine, target) = firearm_fixture(friendly);
        // Use the existing file-loaded synthetic spawn identity for rule binding.
        engine.spawn_catalog = crate::human_kill_reward_tests::fixture().spawn_catalog;
        let mut engine = crate::human_kill_reward_tests::enable_incoming_base(engine, 0.25);
        if friendly {
            // Independent world ticks may retain admitted peers without an active focus.
            let source = engine.active.take().unwrap();
            let source_id = source.reservation.id.clone();
            let attacker = 0x0380_0000 | u32::from(source.reservation.player);
            let world = source.reservation.world;
            engine.peers.insert(source_id.clone(), source);
            assert_eq!(
                engine.character_incoming_base(attacker, 0x0380_0000 | u32::from(target), world,),
                0.25
            );
            engine.active = engine.peers.remove(&source_id);
        }
        let bot = &engine.bot_registry.bots[&target];
        assert_eq!(
            bot.target.faction,
            engine
                .active
                .as_ref()
                .unwrap()
                .reservation
                .character
                .faction
        );
        let wire = crate::human_kill_reward_tests::native_hit(0x0380_0000 | u32::from(target), 23);
        dispatch(&mut engine, &wire, 1000).unwrap();
        assert_eq!(
            engine.bot_registry.bots[&target].health,
            if friendly { 80 } else { 0 }
        );
        assert_eq!(engine.pending_reward_batch().len(), usize::from(!friendly));
    }
}

#[test]
fn opposing_same_faction_native_hit_records_one_shared_bot_life_reward() {
    let (mut engine, target) = firearm_fixture(false);
    let bot = &engine.bot_registry.bots[&target];
    let source = engine.active.as_ref().unwrap();
    assert_ne!(bot.target.squad.team(), source.team());
    assert_eq!(bot.target.faction, source.reservation.character.faction);
    let generation = bot.target.runtime.unwrap().generation;
    let expected = progression::RewardRecipient::from_active(source)
        .unwrap()
        .bot_killed(target, generation);
    native_firearm_hit(&mut engine, target);
    let source = engine.active.as_ref().unwrap();
    assert_eq!(source.reward_statistics.get(&10), Some(&1));
    assert_eq!(source.reward_statistics.get(&14), None);
    assert_eq!(engine.pending_reward_batch(), vec![expected.clone()]);
    // A repeated hit against the dead life cannot add a reward or notification.
    let wire = crate::human_kill_reward_tests::native_hit(0x0380_0000 | u32::from(target), 18);
    dispatch(&mut engine, &wire, 1001).unwrap();
    assert_eq!(engine.pending_reward_batch(), vec![expected]);
    assert_eq!(
        engine.active.as_ref().unwrap().reward_statistics.get(&10),
        Some(&1)
    );
    assert_eq!(
        engine.active.as_ref().unwrap().pending_notifications.len(),
        1
    );
}

#[test]
fn hostile_queued_grenade_records_the_real_human_kill_reward_once_per_bot_life() {
    let (mut engine, target) = fixture();
    let generation = engine.bot_registry.bots[&target]
        .target
        .runtime
        .unwrap()
        .generation;
    queue(&mut engine, target, generation, 0x0380_0000);
    update(&mut engine, 1000).unwrap();
    assert_eq!(engine.bot_registry.bots[&target].health, 0);
    let source = engine.active.as_ref().unwrap();
    assert_eq!(source.reward_statistics.get(&10), Some(&1));
    assert!(source.grenade_damage.is_empty());
    let expected = progression::RewardRecipient::from_active(source)
        .unwrap()
        .event(
            format!(
                "kill-{}-bot-{target:04x}-{generation:08x}",
                source.reservation.id
            ),
            progression::RewardKind::EnemyKilled,
            1,
        );
    let batch = engine.pending_reward_batch();
    assert_eq!(batch, vec![expected.clone()]);
    // Firearms and grenades use the same bot-life receipt; retries cannot add XP.
    let firearm_receipt =
        progression::RewardRecipient::from_active(engine.active.as_ref().unwrap())
            .unwrap()
            .bot_killed(target, generation);
    assert_eq!(firearm_receipt, expected);
    engine.commit_rewards(vec![firearm_receipt]);
    assert_eq!(engine.pending_reward_batch(), vec![expected.clone()]);
    assert_eq!(batch[0].account_id, "synthetic-grenade-owner");
    assert_eq!(batch[0].character_id, 7);
    assert_eq!(batch[0].player_id, 0);
    assert_eq!(
        engine.peers.len(),
        0,
        "the bot must not gain a fake reservation"
    );
    queue(&mut engine, target, generation, 0x0380_0000);
    update(&mut engine, 1001).unwrap();
    assert_eq!(engine.pending_reward_batch(), vec![expected]);
    assert_eq!(
        engine.active.as_ref().unwrap().reward_statistics.get(&10),
        Some(&1)
    );
    // A new represented bot life gets its own identity; an old flight cannot hit it.
    let bot = engine.bot_registry.bots.get_mut(&target).unwrap();
    bot.health = 400;
    bot.damage = initial_damage();
    let runtime = bot.target.runtime.as_mut().unwrap();
    runtime.generation = generation + 1;
    runtime.phase = BotPhase::Complete;
    queue(&mut engine, target, generation, 0x0380_0000);
    update(&mut engine, 1002).unwrap();
    assert_eq!(engine.bot_registry.bots[&target].health, 400);
    assert_eq!(engine.pending_reward_batch().len(), 1);
    queue(&mut engine, target, generation + 1, 0x0380_0000);
    update(&mut engine, 1003).unwrap();
    let batch = engine.pending_reward_batch();
    assert_eq!(batch.len(), 2);
    assert_ne!(batch[0].reward_id, batch[1].reward_id);
    assert_eq!(
        engine.active.as_ref().unwrap().reward_statistics.get(&10),
        Some(&2)
    );
}
#[test]
fn friendly_queued_grenade_preserves_friendly_kill_statistics_without_enemy_xp() {
    let (mut engine, target) = fixture();
    let source_team = engine.active.as_ref().unwrap().team();
    engine
        .bot_registry
        .bots
        .get_mut(&target)
        .unwrap()
        .target
        .squad = replication::SquadAddress::from_index(u16::from(source_team) * 16).unwrap();
    let generation = engine.bot_registry.bots[&target]
        .target
        .runtime
        .unwrap()
        .generation;
    queue(&mut engine, target, generation, 0x0380_0000);
    update(&mut engine, 1000).unwrap();
    assert_eq!(engine.bot_registry.bots[&target].health, 0);
    assert!(engine.pending_reward_batch().is_empty());
    assert_eq!(
        engine.active.as_ref().unwrap().reward_statistics.get(&14),
        Some(&1)
    );
    assert_eq!(
        engine.active.as_ref().unwrap().reward_statistics.get(&10),
        None
    );
}
#[test]
fn a_queued_hit_cannot_credit_an_attacker_that_does_not_own_the_queue() {
    let (mut engine, target) = fixture();
    // Another authenticated actor exists; its handle is not the queue owner's.
    reserve(&mut engine, 1);
    let source = engine.active.as_mut().unwrap();
    ready(source);
    source.life = CharacterLife::Alive {
        generation: 4,
        health_quarter_points: 400,
    };
    source.reservation.character.account_id = Some("synthetic-other-owner".into());
    source.reservation.character.character_id = Some(8);
    let generation = engine.bot_registry.bots[&target]
        .target
        .runtime
        .unwrap()
        .generation;
    queue(&mut engine, target, generation, 0x0380_0000);
    update(&mut engine, 1000).unwrap();
    assert!(engine.pending_reward_batch().is_empty());
    assert_eq!(
        engine.active.as_ref().unwrap().reward_statistics.get(&10),
        None
    );
    assert_eq!(
        engine.peers["synthetic-reservation-0000"]
            .reward_statistics
            .get(&10),
        None
    );
}
#[test]
fn failed_queued_grenade_audit_commits_neither_bot_damage_nor_reward() {
    let (mut engine, target) = fixture();
    let generation = engine.bot_registry.bots[&target]
        .target
        .runtime
        .unwrap()
        .generation;
    queue(&mut engine, target, generation, 0x0380_0000);
    engine.max_log = engine.written;
    assert_eq!(update(&mut engine, 1000).err(), Some("log_limit_reached"));
    assert_eq!(engine.bot_registry.bots[&target].health, 400);
    assert!(engine.pending_reward_batch().is_empty());
    assert_eq!(engine.active.as_ref().unwrap().grenade_damage.len(), 1);
    assert!(engine.active.as_ref().unwrap().reward_statistics.is_empty());
}

#[test]
fn a_grenade_already_queued_before_shooter_death_still_rewards_its_real_launcher() {
    let (mut engine, target) = fixture();
    let generation = engine.bot_registry.bots[&target]
        .target
        .runtime
        .unwrap()
        .generation;
    queue(&mut engine, target, generation, 0x0380_0000);
    // The owner's lethal hit commits first, retaining damage already owed to bots.
    let owner = engine.active.as_mut().unwrap();
    owner.pending_damage = Some(PendingDamage {
        target_character_id: 0x0380_0000,
        target_reservation_id: Some(owner.reservation.id.clone()),
        id: "synthetic-owner-death".into(),
        generation: 4,
        falling: false,
        damage: damage::ResolvedDamage {
            amount_hp: 137.,
            kind: damage::DamageKind::Bullet,
            kinetic_response: false,
            location: 3,
            attacker_id: 0x0380_0000,
            source_object_id: u32::MAX,
            user_data: 0,
        },
    });
    update(&mut engine, 1000).unwrap();
    assert!(matches!(
        engine.active.as_ref().unwrap().life,
        CharacterLife::AwaitingRespawn { generation: 4 }
    ));
    assert_eq!(engine.active.as_ref().unwrap().grenade_damage.len(), 1);
    update(&mut engine, 1001).unwrap();
    assert_eq!(engine.bot_registry.bots[&target].health, 0);
    let rewards = engine.pending_reward_batch();
    assert_eq!(rewards.len(), 1);
    assert_eq!(rewards[0].kind, progression::RewardKind::EnemyKilled);
    assert_eq!(rewards[0].account_id, "synthetic-grenade-owner");
    assert_eq!(rewards[0].reservation_id, "synthetic-reservation-0000");
}
