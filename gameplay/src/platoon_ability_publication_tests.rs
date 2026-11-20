//! Squad command snapshots survive the departure of the last squad member.
use super::*;
use crate::{
    active_abilities::Ability,
    active_ability_engine_tests::{
        activate, actor_mut, add_ready, configured, prepare_effect_tick, request,
    },
};

fn squad_masks(actor: &Active, profile: wire_profile::WireProfile) -> Vec<(u32, u32, bool)> {
    let mut masks = Vec::new();
    for packet in &actor.pending_shared {
        let Ok(update) = replication::ObjectUpdate::decode_with_profile(packet, profile) else {
            continue;
        };
        if update.header.object_type_mask != 1 << 8 {
            continue;
        }
        let mut bits = replication::BitReader::with_profile(update.wire, profile);
        bits.position = profile.header_bits();
        while bits.read(1).unwrap() == 1 {
            let index = bits.read(16).unwrap();
            let mut active = None;
            let mut sensor = false;
            for field in 0..26 {
                if bits.read(1).unwrap() == 0 {
                    continue;
                }
                match field {
                    0 => {
                        bits.read(1).unwrap();
                    }
                    1 | 10 | 23 => {
                        bits.read(16).unwrap();
                    }
                    16..=21 => {
                        for width in [32, 32, 32, 16, 32, 1, 32] {
                            bits.read(width).unwrap();
                        }
                        sensor |= field == 18;
                    }
                    22 => active = Some(bits.read(16).unwrap()),
                    24 => {
                        bits.read(8).unwrap();
                    }
                    _ => panic!("unexpected Squad field {field}"),
                }
            }
            masks.push((index, active.unwrap(), sensor));
        }
    }
    masks
}

#[test]
fn vacated_squad_keeps_active_snapshot_for_late_join_and_publishes_completion() {
    let mut engine = configured();
    add_ready(&mut engine, 0);
    add_ready(&mut engine, 1);
    let profile = engine.wire_profile;
    let observer_squad = std::sync::Arc::new(
        engine
            .spawn_catalog
            .as_ref()
            .unwrap()
            .for_squad(17)
            .unwrap(),
    );
    actor_mut(&mut engine, 1).squad_catalog = observer_squad.clone();
    assert_eq!(actor_mut(&mut engine, 0).squad_catalog.squad_index(), 16);
    assert_eq!(actor_mut(&mut engine, 1).squad_catalog.squad_index(), 17);
    engine.match_started = true;
    engine.reconcile_squad_leaders().unwrap();
    engine
        .queue_active_ability_state(Duration::from_millis(1), 1)
        .unwrap();
    assert_eq!(
        squad_masks(actor_mut(&mut engine, 1), profile),
        [(16, 0, false), (17, 0, false)]
    );
    actor_mut(&mut engine, 1).pending_shared.clear();

    assert!(activate(&mut engine, request(16, Ability::Sensor), 0, 20).ok);
    engine
        .queue_active_ability_state(Duration::from_millis(21), 1)
        .unwrap();
    assert_eq!(
        squad_masks(actor_mut(&mut engine, 1), profile),
        [(16, 1 << 2, true)]
    );
    actor_mut(&mut engine, 1).pending_shared.clear();

    let reservation_id = actor_mut(&mut engine, 0).reservation.id.clone();
    assert!(
        engine
            .handle(
                Request::Release {
                    v: BRIDGE_VERSION,
                    request_id: "synthetic-squad-last-member-release".into(),
                    reservation_id,
                },
                Duration::from_millis(22),
                1
            )
            .ok
    );

    add_ready(&mut engine, 2);
    actor_mut(&mut engine, 2).squad_catalog = observer_squad;
    for player in [1, 2] {
        actor_mut(&mut engine, player).pending_shared.clear();
    }
    engine
        .queue_active_ability_state(Duration::from_millis(23), 1)
        .unwrap();
    assert_eq!(
        squad_masks(actor_mut(&mut engine, 2), profile),
        [(16, 1 << 2, true), (17, 0, false)]
    );
    assert!(squad_masks(actor_mut(&mut engine, 1), profile).is_empty());
    for player in [1, 2] {
        actor_mut(&mut engine, player).pending_shared.clear();
    }

    let mut native = Default::default();
    let tick = prepare_effect_tick(&mut engine, &native, 62).unwrap();
    engine
        .record_acquisition_character_effects(tick, &mut native, 1)
        .unwrap();
    engine
        .queue_active_ability_state(Duration::from_millis(62), 1)
        .unwrap();
    for player in [1, 2] {
        assert_eq!(
            squad_masks(actor_mut(&mut engine, player), profile),
            [(16, 0, true)]
        );
    }
}
