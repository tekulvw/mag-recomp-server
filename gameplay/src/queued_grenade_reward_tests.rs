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
    let bootstraps = super::identity_tests::engine("Synthetic Bot Opponent");
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
