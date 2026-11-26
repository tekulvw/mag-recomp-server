//! Controller objectives publish their authored per-state sequences.
use super::connected_tests::structural_fixture;
use super::*;
use crate::replication::{BitReader, CharacterPosition, ObjectiveSequenceDelta, ObjectiveState};

const CAPTURE: u16 = 100;
const ARRIVAL: u16 = 101;
const STRUCTURAL: u16 = 120;

fn states(pairs: &[(&str, i8)]) -> Value {
    let mut indices: BTreeMap<&str, i8> = [
        "Inactive",
        "Deactivated",
        "Activating",
        "Activated",
        "Deactivating",
        "Completed",
        "Repairing",
        "Repaired",
        "Locked",
        "Retaking",
        "Retaken",
        "CancelRetaken",
    ]
    .into_iter()
    .map(|name| (name, -2))
    .collect();
    indices.extend(pairs.iter().copied());
    json!(indices)
}

fn source(events: &[&str]) -> Value {
    json!({"source_sha256":"b".repeat(64),"events":events})
}

/// A capture that builds its enclosure when deactivated and opens it when
/// completed, and an Arrival with a stopped slot.
fn rows() -> Value {
    json!([
        {"objective_token":format!("0x{CAPTURE:08x}"),"class":"ObjectiveSecureAndHoldProperties",
            "definition_sha256":"c".repeat(64),
            "state_sequence_indices":states(&[("Deactivated", 0), ("Completed", 1)]),
            "sequences":[source(&["AnimObjectCreationEventDef"]),
                source(&["AnimObjectCreationEventDef","AnimEventDef","SoundAttachedEventDef"])]},
        {"objective_token":format!("0x{ARRIVAL:08x}"),"class":"ObjectiveArrivalProperties",
            "definition_sha256":"d".repeat(64),
            "state_sequence_indices":states(&[("Inactive", 0), ("Completed", 1), ("CancelRetaken", -1)]),
            "sequences":[source(&["PlayDependentSequenceEventDef"]),
                source(&["PlayDependentSequenceEventDef","StopSequencesEventDef"])]}
    ])
}

fn fixture() -> crate::Engine<Vec<u8>> {
    structural_fixture(0., 0., 0., 1, Some(rows()))
}

/// Controller selections; the fixture's Structural objective has its own.
fn sequences(engine: &crate::Engine<Vec<u8>>) -> Vec<ObjectiveSequenceDelta> {
    engine
        .acquisition_gameplay
        .as_ref()
        .unwrap()
        .current_structural_sequences()
        .into_iter()
        .filter(|delta| delta.object_index != STRUCTURAL)
        .collect()
}

/// Every standalone fields-9/10 controller row the objective ticks recorded.
fn published(engine: &crate::Engine<Vec<u8>>) -> Vec<(u32, i32, u32)> {
    let mut rows = Vec::new();
    for record in String::from_utf8_lossy(&engine.writer)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|row| row["op"] == "acquisition_objective_tick")
    {
        for packet in record["sequence_updates"].as_array().unwrap() {
            let wire = crate::protocol::decode_hex(packet.as_str().unwrap());
            let mut bits = BitReader::with_profile(&wire, engine.wire_profile);
            bits.position = engine.wire_profile.header_bits();
            while bits.read(1).unwrap() != 0 {
                let index = bits.read(16).unwrap();
                let mut fields = Vec::new();
                for field in 0..11 {
                    if bits.read(1).unwrap() != 0 {
                        fields.push((field, bits.read(32).unwrap()));
                    }
                }
                let [(9, sequence), (10, start)] = fields[..] else {
                    panic!("sequence-only row carries {fields:?}");
                };
                if index != u32::from(STRUCTURAL) {
                    rows.push((index, sequence as i32, start));
                }
            }
        }
    }
    rows
}

fn capture(engine: &mut crate::Engine<Vec<u8>>, now: u64) {
    let actor = engine.active.as_mut().unwrap();
    let position = CharacterPosition {
        x: 100.,
        y: 3.,
        z: 100.,
    };
    actor.tracking.server_position(
        position,
        crate::player_tracking::PositionOrigin::ServerSpawn,
        now,
    );
    let team = actor.team();
    let outcome = engine.objective_world.as_mut().unwrap().interact(
        crate::protocol::ObjectiveInteraction {
            kind: ObjectiveInteractionKind::Activate,
            target_object_id: 0x0c80_0000 | u32::from(CAPTURE),
            actor_index: 0,
            trigger_index: 0,
        },
        ObjectiveActor {
            player: 0,
            team,
            generation: 1,
            position,
        },
        Duration::from_millis(now),
    );
    assert!(outcome.accepted, "{}", outcome.reason);
}

fn tick(engine: &mut crate::Engine<Vec<u8>>, now: u64) {
    engine
        .tick_acquisition_objectives(Duration::from_millis(now), 2)
        .unwrap();
}

#[test]
fn capture_publishes_closed_then_opening_sequence_and_late_join_keeps_its_start() {
    let mut engine = fixture();
    let world = engine.objective_world.as_ref().unwrap();
    assert_eq!(world.state(CAPTURE), Some(ObjectiveState::Deactivated));
    // The match-start Deactivated entry builds the closed enclosure.
    assert_eq!(published(&engine), [(u32::from(CAPTURE), 0, 0)]);
    assert_eq!(
        sequences(&engine),
        [ObjectiveSequenceDelta {
            object_index: CAPTURE,
            sequence_index: 0,
            start_time_ms: 0,
        }]
    );

    capture(&mut engine, 100);
    tick(&mut engine, 150);
    assert_eq!(
        engine.objective_world.as_ref().unwrap().state(CAPTURE),
        Some(ObjectiveState::Activating)
    );
    // Activating keeps the current sequence (-2): nothing new is published.
    assert_eq!(published(&engine).len(), 1);

    tick(&mut engine, 3200);
    let world = engine.objective_world.as_ref().unwrap();
    assert_eq!(world.state(CAPTURE), Some(ObjectiveState::Completed));
    // Arrival's enabled state has no authored slot, so only the opening goes out.
    assert_eq!(
        published(&engine),
        [(u32::from(CAPTURE), 0, 0), (u32::from(CAPTURE), 1, 3200)]
    );
    let opened = ObjectiveSequenceDelta {
        object_index: CAPTURE,
        sequence_index: 1,
        start_time_ms: 3200,
    };
    assert_eq!(sequences(&engine), [opened]);
    // Late-join history encodes the same selection with its original start.
    let batches = crate::replication::ObjectiveSequenceDelta::encode_batches(
        &sequences(&engine),
        9000,
        engine.wire_profile,
    )
    .unwrap();
    assert_eq!(batches.len(), 1);

    // Locked (-2) and later ticks keep the opened selection.
    tick(&mut engine, 9000);
    assert_eq!(sequences(&engine), [opened]);

    // Structural and controller selections share one late-join batch in
    // object-index order.
    let all = engine
        .acquisition_gameplay
        .as_ref()
        .unwrap()
        .current_structural_sequences();
    assert_eq!(
        all.iter()
            .map(|delta| delta.object_index)
            .collect::<Vec<_>>(),
        [CAPTURE, STRUCTURAL]
    );

    engine.reset_objective_world();
    assert!(sequences(&engine).is_empty());
}

#[test]
fn published_selection_skips_keep_unauthored_and_repeated_indices() {
    let engine = fixture();
    let catalog = Catalog::from_json(
        &json!({
            "schema":"mag_domination_structural_effect_inputs_v1","map":"synthetic_arena",
            "metadata_sha256":"0".repeat(64),"level_archive_sha256":"a".repeat(64),
            "level_clv_sha256":"a".repeat(64),"global_data_sha256":"a".repeat(64),
            "physics_sha256":"a".repeat(64),"collision_resources":{},"unresolved_collision_tokens":[],
            "definitions":{"0x000000aa":{"definition_sha256":"a".repeat(64),
                "state_sequence_indices":states(&[]),"sequence_tokens":[],
                "blast_properties":{"SubType":1,"FullDamageRadius":0.,"HalfDamageRadius":0.,
                    "DamageValue":0.,"MaterialPenetration":0.,"CharacterDamageOverTimeRadius":0.,
                    "CharacterDamageOverTimeValue":0.}}},
            "placements":[{"objective_token":"0x000000fe","definition_token":"0x000000aa",
                "pose":{"translation":[0.,0.,0.],"rotation":[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]]}}],
            "sequences":{},"published_sequences":rows()
        })
        .to_string(),
    )
    .unwrap();
    let mut playback =
        Structural::from_acquisition_catalog(catalog, engine.objective_catalog.as_ref().unwrap())
            .unwrap();
    assert_eq!(
        playback.published_objectives().collect::<Vec<_>>(),
        [(CAPTURE, u32::from(CAPTURE)), (ARRIVAL, u32::from(ARRIVAL))]
    );
    use ObjectiveState::*;
    let mut step = |index, state, now| {
        playback
            .prepare_published_sequence(index, state, now)
            .unwrap()
            .map(|delta| (delta.sequence_index, delta.start_time_ms))
    };
    // Unpublished objectives, keep slots, Failed and nothing selected for a
    // stop all publish nothing.
    assert_eq!(step(120, Completed, 1), None);
    assert_eq!(step(CAPTURE, Activating, 1), None);
    assert_eq!(step(CAPTURE, Failed, 1), None);
    assert_eq!(step(ARRIVAL, CancelRetaken, 1), None);
    assert_eq!(step(CAPTURE, Deactivated, 2), Some((0, 2)));
    // A repeated Deactivated entry keeps the running sequence.
    assert_eq!(step(CAPTURE, Deactivated, 3), None);
    assert_eq!(step(CAPTURE, Completed, 4), Some((1, 4)));
    assert_eq!(step(ARRIVAL, Inactive, 5), Some((0, 5)));
    assert_eq!(step(ARRIVAL, CancelRetaken, 6), Some((-1, 6)));
    assert_eq!(
        playback
            .current_sequences()
            .iter()
            .map(|delta| (delta.object_index, delta.sequence_index))
            .collect::<Vec<_>>(),
        [(CAPTURE, 1), (ARRIVAL, -1)]
    );
    playback.reset();
    assert!(playback.current_sequences().is_empty());
}

#[test]
fn published_rows_must_bind_to_distinct_controller_objectives() {
    let engine = fixture();
    let objectives = engine.objective_catalog.as_ref().unwrap();
    let base = |published: Value| {
        json!({
            "schema":"mag_domination_structural_effect_inputs_v1","map":"synthetic_arena",
            "metadata_sha256":"0".repeat(64),"level_archive_sha256":"a".repeat(64),
            "level_clv_sha256":"a".repeat(64),"global_data_sha256":"a".repeat(64),
            "physics_sha256":"a".repeat(64),"collision_resources":{},"unresolved_collision_tokens":[],
            "definitions":{"0x000000aa":{"definition_sha256":"a".repeat(64),
                "state_sequence_indices":states(&[]),"sequence_tokens":[],
                "blast_properties":{"SubType":1,"FullDamageRadius":0.,"HalfDamageRadius":0.,
                    "DamageValue":0.,"MaterialPenetration":0.,"CharacterDamageOverTimeRadius":0.,
                    "CharacterDamageOverTimeValue":0.}}},
            "placements":[{"objective_token":"0x000000fe","definition_token":"0x000000aa",
                "pose":{"translation":[0.,0.,0.],"rotation":[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]]}}],
            "sequences":{},"published_sequences":published
        })
    };
    let row = |token: &str| {
        let mut row = rows()[0].clone();
        row["objective_token"] = json!(token);
        row
    };
    // The catalog itself refuses a placed objective, duplicates, slots past the
    // sequence list, an empty list and a malformed event kind.
    let mut beyond = row("0x00000064");
    beyond["state_sequence_indices"]["Completed"] = json!(2);
    let mut empty = row("0x00000064");
    empty["sequences"] = json!([]);
    empty["state_sequence_indices"] = states(&[]);
    let mut kind = row("0x00000064");
    kind["sequences"][0]["events"] = json!(["Anim Event"]);
    for bad in [
        json!([row("0x000000fe")]),
        json!([row("0x00000064"), row("0x00000064")]),
        json!([beyond]),
        json!([empty]),
        json!([kind]),
    ] {
        assert_eq!(
            Catalog::from_json(&base(bad).to_string()).err(),
            Some("invalid_published_objective_sequences")
        );
    }
    // An unbound objective cannot be published.
    let unbound = Catalog::from_json(&base(json!([row("0x00000999")])).to_string()).unwrap();
    assert_eq!(
        Structural::from_acquisition_catalog(unbound, objectives).err(),
        Some("acquisition_published_sequence_objective_mismatch")
    );
    // The configured controller owns every published objective.
    let runtime = engine.acquisition_gameplay.as_ref().unwrap();
    assert_eq!(runtime.setup.validate_structural(objectives), Ok(()));
}
