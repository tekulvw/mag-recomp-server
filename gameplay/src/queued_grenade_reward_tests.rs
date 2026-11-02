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
