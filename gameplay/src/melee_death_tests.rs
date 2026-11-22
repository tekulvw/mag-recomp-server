//! Human knife-collapse notices through the actual native-event transaction.
//! All equipment, damage amounts and body mappings are synthetic.
use super::*;
use crate::spawn_progression::dispatch;

fn fixture() -> Engine<Vec<u8>> {
    let mut engine = human_kill_reward_tests::fixture();
    engine.item_catalog = human_kill_reward_tests::melee_items();
    for actor in engine.active.iter_mut().chain(engine.peers.values_mut()) {
        actor.owner_fields.yaw = Some(0);
        actor.spawned_loadout.as_mut().unwrap().inventories[0].slots[2] = Some(2);
    }
    engine
}

fn hit(
    engine: &mut Engine<Vec<u8>>,
    now: u64,
    location: u8,
) -> Result<Vec<OutboundMessage>, &'static str> {
    let mut wire = human_kill_reward_tests::native_hit(0x0380_0000, now as u32);
    wire[2] = 2; // Slot-owned knife, independently of the observed firearm.
    wire[4] = location;
    dispatch(engine, &wire, now)
}

fn notices(actor: &Active) -> Vec<String> {
    actor
        .pending_notifications
        .iter()
        .filter_map(|p| match p {
            match_flow::Publication::Application(wire) if wire.starts_with(&[1, 0xff]) => {
                Some(protocol::encode_hex(wire))
            }
            _ => None,
        })
        .collect()
}

fn victim(engine: &Engine<Vec<u8>>) -> &Active {
    &engine.peers["synthetic-reservation-0000"]
}

#[test]
fn human_knife_collapse_publishes_one_weapon_notice_and_keeps_finishing_distinct() {
    let mut engine = fixture();
    let gun = engine.active.as_ref().unwrap().observed_primary;
    hit(&mut engine, 1000, 3).unwrap();
    assert!(matches!(
        victim(&engine).life,
        CharacterLife::Incapacitated { generation: 4, .. }
    ));
    assert_eq!(victim(&engine).last_damage.last_location_and_type, 0x33);
    // ID01ff, attacker1, defender0, Weapon2, signed equipment2, no flags.
    assert_eq!(
        notices(engine.active.as_ref().unwrap()),
        ["01ff00010000020000000200"]
    );
    assert_eq!(notices(victim(&engine)), ["01ff00010000020000000200"]);
    assert_eq!(engine.active.as_ref().unwrap().observed_primary, gun);
    assert_eq!(
        engine.active.as_ref().unwrap().reward_statistics.get(&10),
        Some(&1)
    );
    assert_eq!(victim(&engine).reward_statistics.get(&12), Some(&1));
    assert_eq!(
        engine
            .pending_reward_batch()
            .iter()
            .filter(|r| r.kind == progression::RewardKind::EnemyKilled)
            .count(),
        1
    );
    // Three further 40-HP hits consume the synthetic 83-HP downed budget.
    for now in [1100, 1200, 1300, 1400] {
        hit(&mut engine, now, 3).unwrap();
    }
    assert!(matches!(
        victim(&engine).life,
        CharacterLife::AwaitingRespawn { generation: 4 }
    ));
    assert_eq!(notices(victim(&engine)).len(), 1);
    assert_eq!(notices(engine.active.as_ref().unwrap()).len(), 1);
    assert_eq!(
        engine.active.as_ref().unwrap().reward_statistics.get(&24),
        Some(&1)
    );
    assert_eq!(victim(&engine).reward_statistics.get(&12), Some(&1));
}

#[test]
fn human_knife_nonlethal_and_failed_record_publish_no_notice() {
    let mut engine = fixture();
    engine
        .peers
        .get_mut("synthetic-reservation-0000")
        .unwrap()
        .life = CharacterLife::Alive {
        generation: 4,
        health_quarter_points: 400,
    };
    hit(&mut engine, 1000, 3).unwrap();
    assert!(matches!(
        victim(&engine).life,
        CharacterLife::Alive {
            health_quarter_points: 240,
            ..
        }
    ));
    assert!(notices(victim(&engine)).is_empty());
    assert!(notices(engine.active.as_ref().unwrap()).is_empty());

    let mut failed = fixture();
    failed.max_log = failed.written;
    assert_eq!(hit(&mut failed, 1000, 3).err(), Some("log_limit_reached"));
    assert!(matches!(
        victim(&failed).life,
        CharacterLife::Alive {
            health_quarter_points: 120,
            ..
        }
    ));
    assert!(notices(victim(&failed)).is_empty());
    assert!(notices(failed.active.as_ref().unwrap()).is_empty());
    assert!(failed.pending_reward_batch().is_empty());
}

#[test]
fn human_knife_terminal_head_notice_reaches_only_participant_squads() {
    let mut engine = fixture();
    engine.incapacitation_rules = None;
    for (slot, squad) in [(2, 0), (3, 16), (4, 1)] {
        spawn_progression::reserve(&mut engine, slot);
        let observer = engine.active.as_mut().unwrap();
        spawn_progression::ready(observer);
        observer.squad_catalog = std::sync::Arc::new(
            engine
                .spawn_catalog
                .as_ref()
                .unwrap()
                .for_squad(squad)
                .unwrap(),
        );
    }
    let source = engine.peers.remove("synthetic-reservation-0001").unwrap();
    let observer = engine.active.replace(source).unwrap();
    engine
        .peers
        .insert(observer.reservation.id.clone(), observer);

    hit(&mut engine, 1000, 0).unwrap();
    assert!(matches!(
        victim(&engine).life,
        CharacterLife::AwaitingRespawn { generation: 4 }
    ));
    // The configured head armor region contributes flag0x80, independently of
    // Swipe's damage kind; no valid associated vehicle contributes flag0x40.
    for id in [
        "synthetic-reservation-0000",
        "synthetic-reservation-0002",
        "synthetic-reservation-0003",
    ] {
        assert_eq!(notices(&engine.peers[id]), ["01ff00010000020000000280"]);
    }
    assert_eq!(
        notices(engine.active.as_ref().unwrap()),
        ["01ff00010000020000000280"]
    );
    assert!(notices(&engine.peers["synthetic-reservation-0004"]).is_empty());
    hit(&mut engine, 1100, 0).unwrap();
    assert_eq!(notices(victim(&engine)).len(), 1);
}

#[test]
fn friendly_human_knife_collapse_notifies_once_without_enemy_kill_credit() {
    let mut engine = fixture();
    engine.active.as_mut().unwrap().squad_catalog =
        std::sync::Arc::clone(&victim(&engine).squad_catalog);
    hit(&mut engine, 1000, 3).unwrap();
    assert!(matches!(
        victim(&engine).life,
        CharacterLife::Incapacitated { .. }
    ));
    assert_eq!(notices(victim(&engine)), ["01ff00010000020000000200"]);
    assert_eq!(
        notices(engine.active.as_ref().unwrap()),
        ["01ff00010000020000000200"]
    );
    assert_eq!(
        engine.active.as_ref().unwrap().reward_statistics.get(&14),
        Some(&1)
    );
    assert!(
        !engine
            .pending_reward_batch()
            .iter()
            .any(|r| r.kind == progression::RewardKind::EnemyKilled)
    );
}
