//! Owned skills reach the owner's client (Character field20), so a human medic
//! can revive a downed teammate. Synthetic values only; no extracted catalogs.
use super::*;
use crate::spawn_progression::{dispatch, engine, reserve};
use replication::BitReader;
use wire_profile::WireProfile;

const SKILLS: [u32; 5] = [0x8000_0001, 0x0000_0002, 0x1234_5678, 0, 0xffff_ffff];

fn profile(application_id: u32) -> WireProfile {
    wire_profile::WireProfileCatalog::embedded()
        .profile(application_id)
        .unwrap()
}

/// One Character row as the native field table lays it out: every present
/// field number with its payload width, plus field20's five words.
#[derive(Debug, Default)]
struct Row {
    object: u16,
    present: Vec<usize>,
    state: Option<(u8, [u8; 3])>,
    damage: Option<(u8, u16, u8)>,
    skills: Option<[u32; 5]>,
}

fn skip(bits: &mut BitReader<'_>, mut width: usize) {
    while width > 0 {
        let chunk = width.min(16);
        bits.read(chunk).unwrap();
        width -= chunk;
    }
}

fn read_row(bits: &mut BitReader<'_>, profile: WireProfile) -> Option<Row> {
    if bits.read(1).unwrap() == 0 {
        return None;
    }
    let mut row = Row {
        object: bits.read(16).unwrap() as u16,
        ..Default::default()
    };
    for field in 0..22 {
        if bits.read(1).unwrap() == 0 {
            continue;
        }
        row.present.push(field);
        match field {
            0 => skip(bits, 1),
            1 | 17 => skip(bits, 16),
            3 => {
                bits.read(1 + 8 + 8).unwrap();
                let name = bits.read(16).unwrap() as usize;
                for _ in 0..name + 2 + 2 + 4 + 36 {
                    bits.read(8).unwrap();
                }
            }
            4 => {
                let state = bits.read(8).unwrap() as u8 & 63;
                let user = [0, 1, 2].map(|_| bits.read(8).unwrap() as u8);
                row.state = Some((state, user));
            }
            5 => skip(bits, 49),
            6 => skip(bits, 8),
            7 => skip(bits, 10),
            8 => {
                let kind = bits.read(8).unwrap() as u8 >> 4;
                let health = bits.read(16).unwrap();
                row.damage = Some((kind, (health & 0xfff) as u16, (health >> 12) as u8));
                for _ in 0..3 {
                    bits.read(32).unwrap();
                }
            }
            9 => (),
            11 | 12 => skip(bits, 24),
            13 => skip(bits, 1),
            15 => {
                for _ in 0..5 {
                    bits.read(8).unwrap();
                    for _ in 0..22 {
                        if bits.read(1).unwrap() == 1 {
                            bits.read(profile.inventory_item_bits()).unwrap();
                        }
                    }
                }
            }
            18 => {
                let extension = profile.mag_info_extension().map_or(0, |_| 2);
                for _ in 0..6 + extension {
                    bits.read(8).unwrap();
                }
            }
            19 => skip(bits, 8),
            20 => row.skills = Some([0; 5].map(|_| bits.read(32).unwrap())),
            _ => panic!("unexpected Character field {field}"),
        }
    }
    Some(row)
}

/// Character rows of one update whose mask selects Character first.
fn character_rows(wire: &[u8], profile: WireProfile) -> Vec<Row> {
    let update = replication::ObjectUpdate::decode_with_profile(wire, profile).unwrap();
    assert_ne!(update.header.object_type_mask & (1 << 3), 0);
    assert_eq!(update.header.object_type_mask & 0b111, 0);
    let mut bits = BitReader::with_profile(wire, profile);
    bits.position = profile.header_bits();
    std::iter::from_fn(|| read_row(&mut bits, profile)).collect()
}

fn creation(skills: Option<[u32; 5]>) -> replication::PlayerAndSquad {
    replication::PlayerAndSquad {
        sequence: 1,
        timestamp_ms: 7,
        player: replication::CharacterCreation {
            object_index: 3,
            owner_index: 3,
            info: replication::CharacterInfo {
                team: 0,
                faction: 1,
                character_class: 21,
                name: "Synthetic".into(),
                client_index: 3,
                squad_index: 0,
                account_id: 9,
                np_identity: [0; 36],
            },
            mag_info: replication::MagCharacterInfo {
                nominations: 0,
                level: 2,
                leadership_level: 1,
                skill_specialty: 0,
                head_id: 0,
                voice_id: 0,
            },
            skills,
        },
        squad: replication::SquadCreation {
            object_index: 0,
            owner_index: 0x105,
            members: [
                3,
                u16::MAX,
                u16::MAX,
                u16::MAX,
                u16::MAX,
                u16::MAX,
                u16::MAX,
                u16::MAX,
            ],
            leader_index: 3,
        },
    }
}

#[test]
fn owner_creation_writes_owned_skills_as_field20_words_in_order() {
    for application_id in [20230, 20231] {
        let profile = profile(application_id);
        let wire = creation(Some(SKILLS)).encode_with_profile(profile).unwrap();
        let rows = character_rows(&wire, profile);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].object, 3);
        assert_eq!(rows[0].present, vec![0, 1, 3, 18, 20]);
        assert_eq!(rows[0].skills, Some(SKILLS));
        // Without a skill record the field keeps its constructor zero.
        let bare = creation(None).encode_with_profile(profile).unwrap();
        let rows = character_rows(&bare, profile);
        assert_eq!(rows[0].present, vec![0, 1, 3, 18]);
    }
}

fn owner_with_skills(skills: Option<[u32; 5]>) -> Engine<Vec<u8>> {
    let mut engine = engine();
    reserve(&mut engine, 0);
    let actor = engine.active.as_mut().unwrap();
    actor.reservation.character.skills = skills;
    actor.claimed = Some(Claimed {
        wire: vec![],
        np_identity: [0; 36],
        game_info: protocol::GameInfo {
            client_index: 0,
            level_name: "synthetic_arena".into(),
            start_time_ms: 0,
            attacking_faction: 1,
            defending_faction: 0,
            maximum_points: [1000, 1000],
            current_points: [1000, 1000],
            game_type: 1,
        },
    });
    engine
}

fn client_ready(engine: &mut Engine<Vec<u8>>) -> Vec<u8> {
    let mut bytes = engine
        .wire_profile
        .wire_message_type(0x0005)
        .to_be_bytes()
        .to_vec();
    bytes.push(0x80);
    dispatch(engine, &bytes, 10).unwrap();
    engine
        .active
        .as_ref()
        .unwrap()
        .initial_objects
        .clone()
        .expect("client ready publishes the owner's creation")
}

#[test]
fn the_owners_own_creation_carries_its_skill_record() {
    let mut engine = owner_with_skills(Some(SKILLS));
    let snapshot = client_ready(&mut engine);
    let rows = character_rows(&snapshot, engine.wire_profile);
    assert_eq!(rows[0].object, 0);
    assert!(rows[0].present.contains(&20));
    assert_eq!(rows[0].skills, Some(SKILLS));

    // A character with no stored record keeps the native constructor value.
    let mut engine = owner_with_skills(None);
    let snapshot = client_ready(&mut engine);
    assert_eq!(
        character_rows(&snapshot, engine.wire_profile)[0].skills,
        None
    );
}

#[test]
fn peers_never_receive_another_players_skill_record() {
    let mut engine = owner_with_skills(Some(SKILLS));
    client_ready(&mut engine);
    let actor = engine.active.as_ref().unwrap();
    let snapshot = actor.snapshot(Duration::from_millis(20)).unwrap();
    let rows = character_rows(&snapshot, engine.wire_profile);
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].present.contains(&20), "{:?}", rows[0].present);
}

mod revive_under_large_lobby_flags {
    use super::*;
    use crate::incapacitation_tests::{rules, two_actors_from_engine};

    const PATIENT: &str = "synthetic-reservation-0000";
    const HEALER: &str = "synthetic-reservation-0001";

    fn rows_for(
        engine: &mut Engine<Vec<u8>>,
        messages: Vec<OutboundMessage>,
    ) -> Vec<(String, Row)> {
        let mut out = Vec::new();
        let mut all = messages;
        for _ in 0..64 {
            all.extend(engine.drain_shared(Vec::new(), 1).unwrap());
            if engine
                .peers
                .values()
                .chain(engine.active.iter())
                .all(|actor| actor.pending_shared.is_empty())
            {
                break;
            }
        }
        for message in all {
            let wire = protocol::decode_hex(&message.payload_hex);
            let Ok(update) =
                replication::ObjectUpdate::decode_with_profile(&wire, engine.wire_profile)
            else {
                continue;
            };
            if update.header.object_type_mask != 1 << 3 {
                continue;
            }
            for row in character_rows(&wire, engine.wire_profile) {
                out.push((message.reservation_id.clone(), row));
            }
        }
        out
    }

    #[test]
    fn a_downed_human_reaches_teammates_as_revivable_and_a_human_revive_restores_it() {
        for (coalesce, budget) in [(false, false), (true, false), (true, true)] {
            let mut base = engine().with_incapacitation_rules(rules()).unwrap();
            if coalesce {
                base = base.with_character_replication_coalescing();
            }
            if budget {
                base = base.with_per_recipient_reply_budget();
            }
            let mut engine = two_actors_from_engine(base, 0.5);
            assert_eq!(engine.active.as_ref().unwrap().reservation.id, HEALER);
            let now = Duration::from_millis(500);

            // A server-owned weapon downs the patient: the same planner and the
            // same shared publication a bot shot uses, beside an unrelated
            // Character delta of the same tick that coalescing may join.
            let patient = &engine.peers[PATIENT];
            let hit = replication::CharacterDamage {
                last_location_and_type: 0x13,
                health_quarter_points: 0,
                conditions: 0,
                last_attacker: 0x0380_0064,
                last_source_object: 0x0680_0002,
                last_user_data: 0,
            };
            let (model, life) = engine.plan_human_damage(patient, 40.0, hit, now).unwrap();
            assert!(matches!(life, CharacterLife::Incapacitated { .. }));
            let downed = engine
                .character_damage_update(2, 500, 0, model, true)
                .encode_with_profile(engine.wire_profile)
                .unwrap();
            let fields = player_tracking::ClientCharacterFields {
                position: Some(replication::CharacterPosition {
                    x: 10.,
                    y: 1.,
                    z: 10.,
                }),
                ..Default::default()
            };
            let movement = replication::multiplayer::CharacterOwnerDelta {
                sequence: 2,
                timestamp_ms: 500,
                player: 100,
                fields: &fields,
            }
            .encode_with_profile(engine.wire_profile)
            .unwrap();
            let patient = engine.peers.get_mut(PATIENT).unwrap();
            patient.apply_peer_damage(4, model, now).unwrap();
            patient.life = life;
            engine.broadcast_bot_packet(movement.clone());
            engine.broadcast_bot_packet(downed);
            engine.broadcast_bot_packet(movement);

            let seen = rows_for(&mut engine, Vec::new());
            let healer_view = seen
                .iter()
                .filter(|(id, row)| id == HEALER && row.object == 0)
                .collect::<Vec<_>>();
            assert_eq!(healer_view.len(), 1, "coalesce={coalesce} budget={budget}");
            // Downed and revivable on the teammate's client: forced death
            // state, zero health and the incapacitated condition together.
            assert_eq!(healer_view[0].1.state.map(|s| s.0), Some(7));
            assert_eq!(healer_view[0].1.damage, Some((1, 0, 2)));

            // The medic's revive request is accepted and restores the patient.
            // Through the bridge, so the multiplayer relay fans the restore out.
            let response = engine.handle(
                Request::Event {
                    v: BRIDGE_VERSION,
                    request_id: "synthetic-revive".into(),
                    reservation_id: HEALER.into(),
                    world_id: 1,
                    player_id: 1,
                    transport: Transport::Udp,
                    direction: Direction::ClientToServer,
                    rt_type: 0x3d,
                    plugin_id: Some(1),
                    payload_hex: "021c03800000".into(),
                },
                Duration::from_millis(600),
                1,
            );
            assert!(response.ok, "{:?}", response.error);
            let messages = response.messages;
            assert!(matches!(
                engine.peers[PATIENT].life,
                CharacterLife::Alive {
                    generation: 4,
                    health_quarter_points
                } if health_quarter_points > 0
            ));
            let seen = rows_for(&mut engine, messages);
            for id in [HEALER, PATIENT] {
                let restored = seen
                    .iter()
                    .filter(|(to, row)| to == id && row.object == 0)
                    .filter_map(|(_, row)| row.damage)
                    .collect::<Vec<_>>();
                assert_eq!(
                    restored.len(),
                    1,
                    "{id} coalesce={coalesce} budget={budget}"
                );
                let (kind, health, conditions) = restored[0];
                assert_eq!(kind, 7);
                assert!(health > 0);
                assert_eq!(conditions & 2, 0);
            }
        }
    }
}
