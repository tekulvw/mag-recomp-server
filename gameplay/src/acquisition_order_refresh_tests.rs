//! Acquisition squad orders are sent again only when a squad's primary
//! objective changes. Each resend replays the order's voice line on the
//! client, so an objective changing state elsewhere must leave it alone.
use super::*;

fn events(engine: &Engine<Vec<u8>>) -> Vec<Value> {
    std::str::from_utf8(&engine.writer)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Squad field-12 rows (the HUD order) this engine sent the active human.
fn order_rows(engine: &Engine<Vec<u8>>, replies: &[OutboundMessage]) -> Vec<u16> {
    let squad = engine.active.as_ref().unwrap().squad_catalog.squad_index();
    replies
        .iter()
        .filter_map(|reply| {
            let packet = protocol::decode_hex(&reply.payload_hex);
            let update = ObjectUpdate::decode_with_profile(&packet, engine.wire_profile).ok()?;
            if update.header.object_type_mask != 1 << 8 {
                return None;
            }
            let mut bits = replication::BitReader::with_profile(&packet, engine.wire_profile);
            bits.position = engine.wire_profile.header_bits();
            if bits.read(1).ok()? != 1 || bits.read(16).ok()? != u32::from(squad) {
                return None;
            }
            let mut order = None;
            for field in 0..26 {
                if bits.read(1).ok()? == 1 {
                    if field != 12 {
                        return None;
                    }
                    order = Some(bits.read(16).ok()? as u16);
                }
            }
            order
        })
        .collect()
}

/// Drain the startup order (clear, then the squad's first objective).
fn settle_startup_order(engine: &mut Engine<Vec<u8>>, from_ms: u64) -> Vec<u16> {
    let mut rows = Vec::new();
    for at in from_ms..from_ms + 10 {
        let actor = engine.active.as_ref().unwrap();
        if actor.objective_assigned && !actor.objective_needs_clear {
            break;
        }
        let replies = owner_update(engine, at, 0);
        rows.extend(order_rows(engine, &replies));
    }
    let actor = engine.active.as_ref().unwrap();
    assert!(actor.objective_assigned && !actor.objective_needs_clear);
    rows
}

fn last_tick_changed_objectives(engine: &Engine<Vec<u8>>) -> bool {
    events(engine)
        .iter()
        .rev()
        .find(|event| event["op"] == "acquisition_objective_tick")
        .is_some_and(|tick| !tick["objectives"].as_array().unwrap().is_empty())
}

#[test]
fn acquisition_objective_change_keeps_an_unchanged_squad_order() {
    let mut engine = capture_fixture();
    start_capture(&mut engine, 0, 100);
    assert_eq!(
        settle_startup_order(&mut engine, 101),
        vec![u16::MAX, FIRST_CAPTURE],
        "startup sends the clear and then the squad's first objective"
    );
    // A structure outside every squad's orders is destroyed. Objective state
    // changes, but the squad's primary is still the capture it was sent.
    let world = engine.objective_world.as_mut().unwrap();
    let source = crate::structure_damage::Source {
        player: 0,
        generation: 1,
    };
    assert!(world.stage_precision_strike(120, Some(source)).unwrap());
    let now = 400;
    engine
        .tick_acquisition_objectives(Duration::from_millis(now), now)
        .unwrap();
    assert!(
        last_tick_changed_objectives(&engine),
        "the structure's destruction was not an objective change"
    );
    assert_eq!(
        engine.objective_world.as_ref().unwrap().state(120),
        Some(ObjectiveState::Completed)
    );
    assert_eq!(primary_assignment(&engine), FIRST_CAPTURE);
    let actor = engine.active.as_ref().unwrap();
    assert!(
        actor.objective_assigned,
        "an objective change re-armed the order"
    );
    assert!(
        !actor.objective_needs_clear,
        "an objective change re-armed the order clear"
    );
    for step in 1..=5 {
        let replies = owner_update(&mut engine, now + step, 0);
        assert!(
            order_rows(&engine, &replies).is_empty(),
            "an unchanged order was cleared and sent again"
        );
    }
}

#[test]
fn acquisition_completed_capture_sends_the_squads_new_order_once() {
    let mut engine = capture_fixture();
    start_capture(&mut engine, 0, 100);
    settle_startup_order(&mut engine, 101);
    // Completing the capture moves the squad's primary to its next objective.
    engine
        .tick_acquisition_objectives(Duration::from_millis(3200), 2)
        .unwrap();
    assert_eq!(
        engine
            .objective_world
            .as_ref()
            .unwrap()
            .state(FIRST_CAPTURE),
        Some(ObjectiveState::Completed)
    );
    let primary = primary_assignment(&engine);
    assert_ne!(primary, FIRST_CAPTURE);
    let actor = engine.active.as_ref().unwrap();
    assert!(!actor.objective_assigned && actor.objective_needs_clear);
    let mut rows = Vec::new();
    for step in 1..=10 {
        let replies = owner_update(&mut engine, 3200 + step, 0);
        rows.extend(order_rows(&engine, &replies));
    }
    assert_eq!(rows, vec![u16::MAX, primary]);
    // Later objective ticks that leave the primary alone send nothing more.
    for step in 1..=10_u64 {
        let now = 3300 + step * 50;
        engine
            .tick_acquisition_objectives(Duration::from_millis(now), now)
            .unwrap();
        let replies = owner_update(&mut engine, now + 1, 0);
        assert!(order_rows(&engine, &replies).is_empty());
    }
}
