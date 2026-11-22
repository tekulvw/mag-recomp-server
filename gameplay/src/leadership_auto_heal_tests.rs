//! Connected scheduler tests with synthetic authored leadership and live characters.
use super::*;

fn fixture(modifier: f32, max_modifier: f32) -> Engine<Vec<u8>> {
    fixture_for_vehicle(modifier, max_modifier, false)
}

fn fixture_for_vehicle(modifier: f32, max_modifier: f32, apc_passenger: bool) -> Engine<Vec<u8>> {
    let catalog=leadership_passives::LeadershipCatalog::from_json(&json!({
        "schema_version":1,"application_id":20231,
        "source":{"member":"scripts/character_leadership_abilities.zpr","member_sha256":"a".repeat(64),
        "archive_sha256":"b".repeat(64),"extractor":"synthetic","projector":"synthetic"},
        "max_separation_range":30.,"refresh_hold_ms":3000,"removal_grace_ms":1000,
        "levels":[{"level":0,"name":"none","modifiers":[]},
        {"level":1,"name":"squad","modifiers":[{"kind":"kCharModReloadTime","value":0.1}]},
        {"level":2,"name":"platoon","modifiers":[{"kind":"kCharModAutoRepair","value":1.}]},
        {"level":3,"name":"officer","modifiers":[{"kind":"kCharModAutoHeal","value":modifier}]}]
    }).to_string()).unwrap();
    let policy = leadership_runtime::RolePolicy::from_json(
        &json!({
            "schema_version":1,"application_id":20231,"first_member_squad_leader":false,
            "development_bindings":[{"account_id":1,"character_id":1,"native_role":4}]
        })
        .to_string(),
    )
    .unwrap();
    let mut e=crate::spawn_progression::tests::dynamic_bodies::acquisition_gameplay_tests::automatic_repair_fixture_for_modeled_vehicle(catalog,policy,apc_passenger);
    let actor = e.active.as_mut().unwrap();
    if !apc_passenger {
        actor.ground_seat = None;
        actor.owner_fields.state = Some(replication::CharacterState {
            state: 1,
            stance: 0,
            user_data: [0; 3],
        });
    }
    let skills=skill_catalog::SkillCatalog::from_value(json!({
        "schema_version":2,"application_id":20231,"table":{"Skills":{"MagCharacterSkill0":{"MagCharacterSkill":{
        "Default":{"Modifier":"kCharModHealth","ModifierValue":max_modifier},"Green":{"Available":true}}}}}
    })).unwrap();
    actor.modifiers = skills.resolve(0, &[1, 0, 0, 0, 0]).unwrap();
    set_hp(&mut e, 250);
    e.active.as_mut().unwrap().last_damage = replication::CharacterDamage {
        last_location_and_type: 0x23,
        health_quarter_points: 250,
        conditions: 4,
        last_attacker: 0x0380_0007,
        last_source_object: 0x0680_0002,
        last_user_data: 123,
    };
    e
}
fn set_hp(e: &mut Engine<Vec<u8>>, hp: u16) {
    let a = e.active.as_mut().unwrap();
    a.life = CharacterLife::Alive {
        generation: 1,
        health_quarter_points: hp,
    };
    a.last_damage.health_quarter_points = hp;
}
fn hp(e: &Engine<Vec<u8>>) -> u16 {
    e.active.as_ref().unwrap().last_damage.health_quarter_points
}
fn tick(e: &mut Engine<Vec<u8>>, ms: u64) -> Response {
    let r = e.tick_world(Duration::from_millis(ms), 3, format!("auto-heal-{ms}"));
    assert!(r.ok, "{:?}", r.error);
    r
}
#[test]
fn scheduler_uses_strict_interval_and_deduplicates_transport_updates() {
    let mut e = fixture(5., 0.);
    tick(&mut e, 3200);
    tick(&mut e, 4700);
    assert_eq!(hp(&e), 250);
    tick(&mut e, 4750);
    assert_eq!(hp(&e), 280);
    tick(&mut e, 4750);
    tick(&mut e, 4799);
    assert_eq!(hp(&e), 280);
}
#[test]
fn current_maximum_health_changes_pulse_and_caps_missing_health() {
    let mut e = fixture(5., 0.2);
    tick(&mut e, 3200);
    tick(&mut e, 4750);
    assert_eq!(hp(&e), 286);
    set_hp(&mut e, 478);
    tick(&mut e, 6300);
    assert_eq!(hp(&e), 480);
}
#[test]
fn heal6_preserves_conditions_location_and_original_attribution_without_rewards() {
    let mut e = fixture(5., 0.);
    let before = e.active.as_ref().unwrap().last_damage;
    tick(&mut e, 3200);
    tick(&mut e, 4750);
    let after = e.active.as_ref().unwrap().last_damage;
    assert_eq!(
        (after.health_quarter_points, after.last_location_and_type),
        (280, 0x63)
    );
    assert_eq!(
        (
            after.conditions,
            after.last_attacker,
            after.last_source_object,
            after.last_user_data
        ),
        (
            before.conditions,
            before.last_attacker,
            before.last_source_object,
            before.last_user_data
        )
    );
    assert!(matches!(
        e.active.as_ref().unwrap().life,
        CharacterLife::Alive {
            health_quarter_points: 280,
            ..
        }
    ));
    assert!(e.pending_rewards.is_empty());
    assert!(e.active.as_ref().unwrap().healing.is_none());
}

#[test]
fn delayed_scheduler_applies_one_pulse_and_retains_remainder() {
    let mut e = fixture(5., 0.);
    tick(&mut e, 3200);
    tick(&mut e, 8200);
    assert_eq!(hp(&e), 280);
    tick(&mut e, 8250);
    assert_eq!(hp(&e), 310);
    tick(&mut e, 8300);
    assert_eq!(hp(&e), 340);
    tick(&mut e, 8350);
    assert_eq!(hp(&e), 340);
}
#[test]
fn full_health_consumes_the_interval_without_applying_heal6() {
    let mut e = fixture(5., 0.);
    set_hp(&mut e, 400);
    tick(&mut e, 3200);
    tick(&mut e, 4750);
    assert_eq!(
        e.active
            .as_ref()
            .unwrap()
            .last_damage
            .last_location_and_type,
        0x23
    );
    set_hp(&mut e, 250);
    tick(&mut e, 4800);
    assert_eq!(hp(&e), 250);
    tick(&mut e, 6250);
    assert_eq!(hp(&e), 280);
}
#[test]
fn admitted_sub_quarter_and_fractional_maximum_pulses_still_publish_heal6() {
    for (modifier, max_modifier, start) in [(0.01, 0., 250), (5., 0.001, 400)] {
        let mut e = fixture(modifier, max_modifier);
        set_hp(&mut e, start);
        tick(&mut e, 3200);
        let r = tick(&mut e, 4750);
        assert_eq!(hp(&e), start);
        assert_eq!(
            e.active
                .as_ref()
                .unwrap()
                .last_damage
                .last_location_and_type,
            0x63
        );
        assert!(r.messages.iter().any(|m| {
            damage_field(&protocol::decode_hex(&m.payload_hex), e.wire_profile)
                .is_some_and(|(_, d)| d.0 == 0x63)
        }));
    }
}
#[test]
fn epsilon_boundary_is_strict_and_negative_amount_cannot_damage() {
    for (modifier, want_type) in [(0.00049, 0x23), (0.0005, 0x63), (-5., 0x23)] {
        let mut e = fixture(modifier, 0.);
        tick(&mut e, 3200);
        tick(&mut e, 4750);
        assert_eq!(hp(&e), 250);
        assert_eq!(
            e.active
                .as_ref()
                .unwrap()
                .last_damage
                .last_location_and_type,
            want_type
        );
    }
}
#[test]
fn new_life_round_and_connection_do_not_inherit_an_old_timer() {
    for boundary in 0..3 {
        let mut e = fixture(5., 0.);
        tick(&mut e, 3200);
        tick(&mut e, 4600);
        let a = e.active.as_mut().unwrap();
        match boundary {
            0 => {
                a.life = CharacterLife::Alive {
                    generation: 2,
                    health_quarter_points: 250,
                }
            }
            1 => a.reward_match_id = "synthetic-next-round".into(),
            _ => a.reservation.id = "synthetic-reconnect".into(),
        }
        tick(&mut e, 4700);
        tick(&mut e, 4750);
        assert_eq!(hp(&e), 250);
        tick(&mut e, 6250);
        assert_eq!(hp(&e), 280);
    }
}
#[test]
fn unsupported_lives_and_mounts_never_heal() {
    for boundary in 0..8 {
        let mut e = fixture(5., 0.);
        tick(&mut e, 3200);
        tick(&mut e, 4600);
        let a = e.active.as_mut().unwrap();
        match boundary {
            0 => a.life = CharacterLife::AwaitingRespawn { generation: 1 },
            1 => a.last_damage.conditions |= 2,
            2 => {
                a.ground_seat = Some(ground_seats::PassengerSeat {
                    vehicle_index: 4,
                    seat: 0,
                })
            }
            3 => a.mounted_seat = Some(0),
            4 => a.ready_for_updates = false,
            5 => {
                a.life = CharacterLife::Incapacitated {
                    generation: 1,
                    downed: incapacitation::Downed {
                        deadline: Duration::from_secs(20),
                        accumulated_damage_hp: 0.,
                    },
                }
            }
            6 => a.life = CharacterLife::AwaitingFirstSpawn,
            _ => a.owner_fields.state.as_mut().unwrap().state = 14,
        }
        e.tick_leadership_auto_heal(Duration::from_millis(4750), 3)
            .unwrap();
        assert_eq!(hp(&e), 250);
    }
}

fn mount_for_healing(engine: &mut Engine<Vec<u8>>, vehicle: u16, seat: u8, now: u64) {
    if seat == 1 {
        crate::ground_turrets::tests::opt_in(engine, vehicle);
    }
    let position = engine.current_ground_vehicle(vehicle).unwrap().position;
    let actor = engine.active.as_mut().unwrap();
    actor
        .tracking
        .server_position(position, player_tracking::PositionOrigin::ServerSpawn, now);
    let character = 0x0380_0000 | u32::from(actor.reservation.player);
    let mut wire = 0x0216u16.to_be_bytes().to_vec();
    wire.extend((0x0100_0000 | u32::from(vehicle)).to_be_bytes());
    wire.extend(u32::from(seat).to_be_bytes());
    wire.extend(character.to_be_bytes());
    crate::spawn_progression::dispatch(engine, &wire, now).unwrap();
    assert_eq!(
        engine.active.as_ref().unwrap().ground_seat,
        Some(ground_seats::PassengerSeat {
            vehicle_index: vehicle,
            seat,
        })
    );
}

#[test]
fn qualified_ground_occupants_receive_character_auto_heal_without_healing_credit() {
    for (apc, seat) in [(false, 0), (false, 1), (false, 2), (true, 2)] {
        let mut engine = fixture_for_vehicle(5., 0.2, apc);
        let vehicle = if apc { 0 } else { 4 };
        if !apc {
            mount_for_healing(&mut engine, vehicle, seat, 3240);
        }
        let before = engine.active.as_ref().unwrap().last_damage;
        let statistics = engine.active.as_ref().unwrap().reward_statistics.clone();
        let association = engine.active.as_ref().unwrap().ground_seat;
        engine
            .tick_leadership_auto_heal(Duration::from_millis(3250), 3)
            .unwrap();
        engine
            .tick_leadership_auto_heal(Duration::from_millis(4750), 3)
            .unwrap();
        assert_eq!(hp(&engine), 250);
        engine
            .tick_leadership_auto_heal(Duration::from_millis(4800), 3)
            .unwrap();
        let actor = engine.active.as_ref().unwrap();
        assert_eq!(hp(&engine), 286, "APC {apc}, seat {seat}");
        assert_eq!(actor.ground_seat, association);
        assert_eq!(actor.owner_fields.state.unwrap().state, 14);
        assert_eq!(actor.last_damage.last_location_and_type, 0x63);
        assert_eq!(actor.last_damage.conditions, before.conditions);
        assert_eq!(actor.last_damage.last_attacker, before.last_attacker);
        assert_eq!(
            actor.last_damage.last_source_object,
            before.last_source_object
        );
        assert_eq!(actor.last_damage.last_user_data, before.last_user_data);
        assert!(engine.pending_rewards.is_empty());
        assert_eq!(actor.reward_statistics, statistics);
        assert!(actor.healing.is_none());
    }
}

#[test]
fn mounting_does_not_restart_the_same_characters_auto_heal_interval() {
    let mut engine = fixture(5., 0.);
    tick(&mut engine, 3200);
    tick(&mut engine, 4600);
    mount_for_healing(&mut engine, 4, 0, 4610);
    engine
        .tick_leadership_auto_heal(Duration::from_millis(4750), 3)
        .unwrap();
    assert_eq!(hp(&engine), 280);
    let actor = engine.active.as_mut().unwrap();
    actor.ground_seat = None;
    actor.owner_fields.state = Some(replication::CharacterState {
        state: 1,
        stance: 0,
        user_data: [0; 3],
    });
    engine
        .tick_leadership_auto_heal(Duration::from_millis(6250), 3)
        .unwrap();
    assert_eq!(hp(&engine), 310);
}

#[test]
fn mounted_auto_heal_keeps_record_refusal_and_stale_gunner_fences() {
    let mut engine = fixture(5., 0.);
    mount_for_healing(&mut engine, 4, 1, 3240);
    engine
        .tick_leadership_auto_heal(Duration::from_millis(3250), 3)
        .unwrap();
    let before = engine.automatic_heal.clone();
    let pending = engine.active.as_ref().unwrap().pending_shared.clone();
    engine.max_log = engine.written;
    assert_eq!(
        engine.tick_leadership_auto_heal(Duration::from_millis(4800), 3),
        Err("log_limit_reached")
    );
    assert_eq!(hp(&engine), 250);
    assert_eq!(engine.automatic_heal, before);
    assert_eq!(engine.active.as_ref().unwrap().pending_shared, pending);

    let mut engine = fixture(5., 0.);
    mount_for_healing(&mut engine, 4, 1, 3240);
    engine
        .tick_leadership_auto_heal(Duration::from_millis(3250), 3)
        .unwrap();
    engine.active.as_mut().unwrap().life = CharacterLife::Alive {
        generation: 2,
        health_quarter_points: 250,
    };
    engine
        .tick_leadership_auto_heal(Duration::from_millis(4800), 3)
        .unwrap();
    engine
        .tick_leadership_auto_heal(Duration::from_millis(6350), 3)
        .unwrap();
    assert_eq!(hp(&engine), 250);
}
#[test]
fn failed_record_preserves_clock_model_cache_and_queue_before_retry() {
    let mut e = fixture(5., 0.);
    e.tick_leadership_auto_heal(Duration::from_millis(3200), 3)
        .unwrap();
    let before = e.automatic_heal.clone();
    let queue = e.active.as_ref().unwrap().pending_shared.clone();
    let cache = format!("{:?}", e.active.as_ref().unwrap().leadership);
    let max = e.max_log;
    e.max_log = e.written;
    assert_eq!(
        e.tick_leadership_auto_heal(Duration::from_millis(4750), 3),
        Err("log_limit_reached")
    );
    assert_eq!(e.automatic_heal, before);
    assert_eq!(hp(&e), 250);
    assert_eq!(e.active.as_ref().unwrap().pending_shared, queue);
    assert_eq!(
        format!("{:?}", e.active.as_ref().unwrap().leadership),
        cache
    );
    e.failed = false;
    e.max_log = max;
    e.tick_leadership_auto_heal(Duration::from_millis(4750), 3)
        .unwrap();
    assert_eq!(hp(&e), 280);
    e.tick_leadership_auto_heal(Duration::from_millis(4750), 3)
        .unwrap();
    assert_eq!(hp(&e), 280);
}
#[test]
fn provider_loss_preserves_hold_then_expiry_resets_the_timer() {
    let mut e = fixture(5., 0.);
    let squad = std::sync::Arc::clone(&e.active.as_ref().unwrap().squad_catalog);
    spawn_progression::tests::reserve(&mut e, 1);
    let a = e.active.as_mut().unwrap();
    spawn_progression::tests::ready(a);
    a.squad_catalog = squad;
    a.owner_fields.state = Some(replication::CharacterState {
        state: 1,
        stance: 0,
        user_data: [0; 3],
    });
    set_hp(&mut e, 250);
    e.tick_leadership_auto_heal(Duration::from_millis(3200), 3)
        .unwrap();
    e.peers
        .values_mut()
        .find(|a| a.reservation.player == 0)
        .unwrap()
        .owner_fields
        .state
        .as_mut()
        .unwrap()
        .state = 0;
    e.tick_leadership_auto_heal(Duration::from_millis(4750), 3)
        .unwrap();
    assert_eq!(hp(&e), 280);
    e.tick_leadership_auto_heal(Duration::from_millis(6250), 3)
        .unwrap();
    assert_eq!(hp(&e), 310);
    e.tick_leadership_auto_heal(Duration::from_millis(7200), 3)
        .unwrap();
    e.tick_leadership_auto_heal(Duration::from_millis(8250), 3)
        .unwrap();
    assert_eq!(hp(&e), 310);
}
#[test]
fn reset_discards_clocks_and_quiet_ticks_do_not_fill_the_log() {
    let mut e = fixture(0., 0.);
    e.tick_leadership_auto_heal(Duration::from_millis(3200), 3)
        .unwrap();
    let before = e.written;
    for ms in (3250..=4700).step_by(50) {
        e.tick_leadership_auto_heal(Duration::from_millis(ms), 3)
            .unwrap();
    }
    assert_eq!(e.written, before);
    e.reset_objective_world();
    assert_eq!(e.automatic_heal, leadership_auto_heal::State::default());
}

// Decode actual field8 contents for both delta and full character snapshots.
type DamageField = (u16, (u32, u32, u32, u32, u32));
fn damage_field(wire: &[u8], profile: wire_profile::WireProfile) -> Option<DamageField> {
    let update = replication::ObjectUpdate::decode_with_profile(wire, profile).ok()?;
    if update.header.object_type_mask != 8 {
        return None;
    }
    let mut b = replication::BitReader::with_profile(wire, profile);
    b.position = profile.header_bits();
    if b.read(1).ok()? == 0 {
        return None;
    }
    let index = b.read(16).ok()? as u16;
    for field in 0..=8 {
        if b.read(1).ok()? == 0 {
            continue;
        }
        match field {
            0 => {
                b.read(1).ok()?;
            }
            1 => {
                b.read(16).ok()?;
            }
            3 => {
                b.read(1).ok()?;
                b.read(8).ok()?;
                b.read(8).ok()?;
                let len = b.read(16).ok()?;
                for _ in 0..len {
                    b.read(8).ok()?;
                }
                for n in [16, 16, 32] {
                    b.read(n).ok()?;
                }
                for _ in 0..36 {
                    b.read(8).ok()?;
                }
            }
            4 => {
                b.read(32).ok()?;
            }
            5 => {
                b.read(32).ok()?;
                b.read(17).ok()?;
            }
            6 => {
                b.read(8).ok()?;
            }
            7 => {
                b.read(10).ok()?;
            }
            8 => {
                return Some((
                    index,
                    (
                        b.read(8).ok()?,
                        b.read(16).ok()?,
                        b.read(32).ok()?,
                        b.read(32).ok()?,
                        b.read(32).ok()?,
                    ),
                ));
            }
            _ => return None,
        }
    }
    None
}
#[test]
fn owner_current_loading_and_late_peer_receive_the_committed_model() {
    let mut e = fixture(5., 0.);
    let a = e.active.as_mut().unwrap();
    a.spawn_delta = Some(vec![]);
    a.claimed = Some(Claimed {
        wire: vec![],
        np_identity: [0; 36],
        game_info: protocol::GameInfo {
            client_index: 0,
            level_name: "synthetic_arena".into(),
            start_time_ms: 0,
            attacking_faction: 0,
            defending_faction: 1,
            maximum_points: [20, 20],
            current_points: [20, 20],
            game_type: 2,
        },
    });
    e.tick_leadership_auto_heal(Duration::from_millis(3200), 3)
        .unwrap();
    spawn_progression::tests::reserve(&mut e, 1);
    spawn_progression::tests::ready(e.active.as_mut().unwrap());
    e.active.as_mut().unwrap().ready_for_updates = false;
    for a in e.active.iter_mut().chain(e.peers.values_mut()) {
        a.pending_shared.clear();
    }
    e.tick_leadership_auto_heal(Duration::from_millis(4750), 3)
        .unwrap();
    let expected = (0, (0x63, 280 | (4 << 12), 0x0380_0007, 0x0680_0002, 123));
    for a in e.active.iter().chain(e.peers.values()) {
        assert!(
            a.pending_shared
                .iter()
                .any(|p| damage_field(p, e.wire_profile) == Some(expected))
        );
    }
    let owner = e
        .peers
        .values()
        .find(|a| a.reservation.player == 0)
        .unwrap();
    assert_eq!(
        damage_field(
            &owner.snapshot(Duration::from_millis(4800)).unwrap(),
            e.wire_profile
        ),
        Some(expected)
    );
}

#[test]
fn passive_preserves_an_independent_item_effect_and_its_cooldown() {
    let mut e = fixture(5., 0.);
    let actor = e.active.as_mut().unwrap();
    actor.life = CharacterLife::Alive {
        generation: 4,
        health_quarter_points: 250,
    };
    actor.healing = Some(progression::HealingSession {
        effect: crate::bot_registry::healing_reward_relationship_tests::effect(),
        healer: None,
        effect_id: "synthetic-item-session".into(),
    });
    actor.next_heal_allowed = Duration::from_secs(10);
    e.tick_leadership_auto_heal(Duration::from_millis(3200), 3)
        .unwrap();
    e.tick_leadership_auto_heal(Duration::from_millis(4750), 3)
        .unwrap();
    assert_eq!(hp(&e), 280);
    let a = e.active.as_ref().unwrap();
    let effect = a.healing.as_ref().unwrap();
    assert_eq!(effect.effect_id, "synthetic-item-session");
    assert_eq!(effect.generation, 4);
    assert!(effect.is_due(Duration::from_secs(1)));
    assert_eq!(a.next_heal_allowed, Duration::from_secs(10));
    assert!(e.pending_rewards.is_empty());
}

#[test]
fn queued_passive_health_precedes_a_newer_item_model_on_the_same_reliable_stream() {
    let mut e = fixture(5., 0.);
    let a = e.active.as_mut().unwrap();
    a.life = CharacterLife::Alive {
        generation: 4,
        health_quarter_points: 250,
    };
    a.healing = Some(progression::HealingSession {
        effect: crate::bot_registry::healing_reward_relationship_tests::effect(),
        healer: None,
        effect_id: "synthetic-backlogged-item".into(),
    });
    a.pending_shared.clear();
    e.tick_leadership_auto_heal(Duration::from_millis(3200), 3)
        .unwrap();
    // A due pulse can remain in this FIFO when the preceding world reply has
    // exhausted its normal publication budget. Do not discard those bytes.
    e.tick_leadership_auto_heal(Duration::from_millis(4750), 3)
        .unwrap();
    let a = e.active.as_ref().unwrap();
    let wire = replication::ObjectUpdate {
        header: replication::UpdateHeader {
            sequence: a.latest_owner_sequence + 1,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 0,
            timestamp_ms: 4800,
        },
        wire: &[],
    }
    .encode_empty_with_profile(e.wire_profile)
    .unwrap();
    let request = Request::Event {
        v: BRIDGE_VERSION,
        request_id: "synthetic-ordered-heal".into(),
        reservation_id: a.reservation.id.clone(),
        world_id: a.reservation.world,
        player_id: a.reservation.player,
        transport: Transport::Udp,
        direction: Direction::ClientToServer,
        rt_type: 0x3d,
        plugin_id: Some(1),
        payload_hex: protocol::encode_hex(&wire),
    };
    let r = e.handle(request, Duration::from_millis(4800), 3);
    assert!(r.ok, "{:?}", r.error);
    assert_eq!(hp(&e), 320);
    let models: Vec<_> = r
        .messages
        .iter()
        .filter(|m| m.player_id == 0)
        .filter_map(|m| {
            let wire = protocol::decode_hex(&m.payload_hex);
            damage_field(&wire, e.wire_profile).map(|(_, d)| {
                (
                    replication::ObjectUpdate::decode_with_profile(&wire, e.wire_profile)
                        .unwrap()
                        .header
                        .sequence,
                    d.1 & 0xfff,
                )
            })
        })
        .collect();
    assert_eq!(
        models.iter().map(|(_, hp)| *hp).collect::<Vec<_>>(),
        vec![280, 320]
    );
    assert!(models.windows(2).all(|p| p[0].0 < p[1].0));
}

fn queue_damage(e: &mut Engine<Vec<u8>>, ordinal: u32) {
    let a = e.active.as_mut().unwrap();
    a.pending_damage = Some(PendingDamage {
        target_character_id: 0x0380_0000,
        target_reservation_id: Some(a.reservation.id.clone()),
        id: format!("synthetic-ordered-damage-{ordinal}"),
        generation: 1,
        source: PendingDamageSource::Falling,
        damage: damage::ResolvedDamage {
            amount_hp: 10.,
            kind: damage::DamageKind::ExternalKinetic,
            kinetic_response: false,
            location: 0,
            attacker_id: 0x0380_0000,
            source_object_id: u32::MAX,
            user_data: 0,
        },
    });
}
fn dispatch_update(e: &mut Engine<Vec<u8>>, ms: u64) -> Vec<OutboundMessage> {
    let wire = replication::ObjectUpdate {
        header: replication::UpdateHeader {
            sequence: e.active.as_ref().unwrap().latest_owner_sequence + 1,
            nak_sequence: 0,
            client_index: 0,
            object_type_mask: 0,
            timestamp_ms: ms as u32,
        },
        wire: &[],
    }
    .encode_empty_with_profile(e.wire_profile)
    .unwrap();
    spawn_progression::tests::dispatch(e, &wire, ms).unwrap()
}
#[test]
fn partial_drain_keeps_later_damage_ordered_after_the_original_heal_leaves_the_fifo() {
    let mut e = fixture(5., 0.);
    e.maximum_players = 8;
    e.active.as_mut().unwrap().pending_shared.clear();
    e.tick_leadership_auto_heal(Duration::from_millis(3200), 3)
        .unwrap();
    e.tick_leadership_auto_heal(Duration::from_millis(4750), 3)
        .unwrap();
    queue_damage(&mut e, 1);
    assert!(dispatch_update(&mut e, 4800).is_empty());
    assert_eq!(hp(&e), 240);
    let a = e.active.as_ref().unwrap();
    let budget: Vec<_> = (0..31)
        .map(|_| OutboundMessage {
            reservation_id: a.reservation.id.clone(),
            world_id: 1,
            player_id: 0,
            plugin_id: 1,
            payload_hex: "0000".into(),
        })
        .collect();
    let first = e.drain_shared(budget, 3).unwrap();
    assert_eq!(first.len(), 32);
    // The committed inherited tier joins the FIFO after the existing heal and
    // damage frames. Only one of the two remaining packets carries health.
    let pending = &e.active.as_ref().unwrap().pending_shared;
    assert_eq!(pending.len(), 2);
    assert_eq!(
        pending
            .iter()
            .filter(|wire| damage_field(wire, e.wire_profile).is_some())
            .count(),
        1
    );
    queue_damage(&mut e, 2);
    assert!(dispatch_update(&mut e, 4850).is_empty());
    assert_eq!(hp(&e), 200);
    let rest = e.drain_shared(Vec::new(), 3).unwrap();
    let health: Vec<_> = rest
        .iter()
        .filter_map(|m| {
            damage_field(&protocol::decode_hex(&m.payload_hex), e.wire_profile)
                .map(|(_, d)| d.1 & 0xfff)
        })
        .collect();
    assert_eq!(health, vec![240, 200]);
    assert!(!e.active.as_ref().unwrap().ordered_health_pending);
    queue_damage(&mut e, 3);
    assert!(!dispatch_update(&mut e, 4900).is_empty());
    assert_eq!(hp(&e), 160);
}
