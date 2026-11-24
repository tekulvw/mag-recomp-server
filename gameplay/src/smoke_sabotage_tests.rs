//! Smoke thrown in a direct Sabotage world, from either equipment gear.
use super::*;
use crate::{
    hospital_rocket_runtime::tests::launch_wire, replication::ObjectUpdate,
    smoke_runtime_tests::owner_terminal,
};
use std::sync::Arc;

const OBSERVER: &str = "synthetic-reservation-0000";
const THROWER: &str = "synthetic-reservation-0001";
const SMOKE_ITEM: u16 = 451;
const FRAG_ITEM: u16 = 4;
const SLOT: u8 = 3;
const VOLUME_MS: u64 = 750;

/// Every packet each human was sent, through direct replies and the shared
/// reliable queues, in arrival order.
#[derive(Default)]
struct Delivered(std::collections::BTreeMap<String, Vec<Vec<u8>>>);

impl Delivered {
    fn take(&mut self, messages: Vec<OutboundMessage>) {
        for message in messages {
            self.0
                .entry(message.reservation_id)
                .or_default()
                .push(protocol::decode_hex(&message.payload_hex));
        }
    }

    /// One native event through the full bridge path, then a publication pass
    /// and shared drains as the world tick performs them.
    fn event(&mut self, engine: &mut Engine<Vec<u8>>, bytes: &[u8], now_ms: u64) -> Response {
        let response = engine.handle(
            Request::Event {
                v: BRIDGE_VERSION,
                request_id: format!("synthetic-smoke-{now_ms}"),
                reservation_id: THROWER.into(),
                world_id: 1,
                player_id: 1,
                transport: Transport::Udp,
                direction: Direction::ClientToServer,
                rt_type: 0x3d,
                plugin_id: Some(1),
                payload_hex: protocol::encode_hex(bytes),
            },
            Duration::from_millis(now_ms),
            1,
        );
        assert!(response.ok, "{:?}", response.error);
        self.take(response.messages.clone());
        self.publish(engine, now_ms);
        response
    }

    fn publish(&mut self, engine: &mut Engine<Vec<u8>>, now_ms: u64) {
        engine
            .queue_projectile_publications(Duration::from_millis(now_ms))
            .unwrap();
        for _ in 0..16 {
            let messages = engine.drain_shared(Vec::new(), 1).unwrap();
            if messages.is_empty() {
                break;
            }
            self.take(messages);
        }
    }

    fn count(&self, engine: &Engine<Vec<u8>>, who: &str, test: impl Fn(&[u8]) -> bool) -> usize {
        let _ = engine;
        self.0
            .get(who)
            .map_or(0, |packets| packets.iter().filter(|p| test(p)).count())
    }
}

fn actor<'a>(engine: &'a Engine<Vec<u8>>, id: &str) -> &'a Active {
    engine
        .active
        .iter()
        .chain(engine.peers.values())
        .find(|actor| actor.reservation.id == id)
        .unwrap()
}

/// Two humans in a game-type-0 retail world. The thrower carries a frag in
/// gear 4 and smoke in gear 5, the placement a saved retail kit allows.
fn sabotage_fixture() -> Engine<Vec<u8>> {
    let mut engine = human_kill_reward_tests::fixture();
    engine.item_catalog = human_kill_reward_tests::grenade_items_with_cover();
    assert_eq!(engine.match_profile().game_type, 0);
    assert!(engine.acquisition_gameplay.is_none() && engine.domination_gameplay.is_none());
    let actor = engine.active.as_mut().unwrap();
    let faction = actor.reservation.character.faction;
    let kit = actor.spawned_loadout.as_mut().unwrap();
    kit.inventories[0].slots[4] = Some(FRAG_ITEM);
    kit.inventories[0].slots[5] = Some(SMOKE_ITEM);
    actor.grenade_remaining = 1;
    actor.gear5_grenade_remaining = 2;
    actor.life = CharacterLife::Alive {
        generation: 4,
        health_quarter_points: 400,
    };
    let smoke = std::array::from_fn(|index| smoke_grenades::Variant {
        faction: index as u8,
        item_id: if index == usize::from(faction) {
            SMOKE_ITEM
        } else {
            SMOKE_ITEM + 1 + index as u16
        },
        ammo_token: 0x1fe2_3480 + index as u32,
        initial_count: 2,
        fuse_ms: 100,
        projectile_lifetime_ms: 10000,
        volume_duration_ms: VOLUME_MS,
    });
    engine
        .with_smoke_grenades(Arc::new(smoke_grenades::Catalog::synthetic(smoke)))
        .expect("Sabotage accepts the smoke capability")
}

fn mask(packet: &[u8], engine: &Engine<Vec<u8>>) -> Option<u32> {
    ObjectUpdate::decode_with_profile(packet, engine.wire_profile)
        .ok()
        .map(|update| update.header.object_type_mask)
}

fn projectile_reports(
    packet: &[u8],
    engine: &Engine<Vec<u8>>,
) -> Vec<projectile::ProjectileOwnerUpdate> {
    ObjectUpdate::decode_with_profile(packet, engine.wire_profile)
        .ok()
        .and_then(|update| {
            projectile::decode_projectile_updates_with_profile(&update, engine.wire_profile).ok()
        })
        .unwrap_or_default()
}

fn is_creation(packet: &[u8], engine: &Engine<Vec<u8>>) -> bool {
    mask(packet, engine) == Some(1 << (16 + SLOT)) && projectile_reports(packet, engine).is_empty()
}

fn is_state_two(packet: &[u8], engine: &Engine<Vec<u8>>) -> bool {
    projectile_reports(packet, engine)
        .iter()
        .any(|report| report.slot == SLOT && report.fields.state == Some(2))
}

fn is_volume_creation(packet: &[u8], engine: &Engine<Vec<u8>>) -> bool {
    mask(packet, engine) == Some(1 << 5)
        && smoke_volume_replication::is_removal(packet, engine.wire_profile).is_none()
}

fn denials(engine: &Engine<Vec<u8>>) -> Vec<String> {
    std::str::from_utf8(&engine.writer)
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|entry| entry["gameplay_denial"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn sabotage_smoke_from_gear_five_reaches_the_other_human_with_its_authored_lifetime() {
    let mut engine = sabotage_fixture();
    let launch = launch_wire(&engine, u16::from(SLOT), 5);
    let terminal = owner_terminal(&engine, 1100);
    let mut wire = Delivered::default();
    wire.event(&mut engine, &launch, 1000);
    let thrower = actor(&engine, THROWER);
    assert_eq!(thrower.gear5_grenade_remaining, 1);
    assert_eq!(
        thrower.grenade_remaining, 1,
        "gear 4 frag stock is untouched"
    );
    assert!(thrower.projectiles.get(SLOT).is_some_and(|f| !f.completed));
    // Flight: the other human receives the projectile creation.
    assert_eq!(
        wire.count(&engine, OBSERVER, |p| is_creation(p, &engine)),
        1
    );

    wire.event(&mut engine, &terminal, 1100);
    let thrower = actor(&engine, THROWER);
    let volumes = thrower.smoke_volumes.volumes();
    assert_eq!(volumes.len(), 1);
    assert_eq!(volumes[0].expires, Duration::from_millis(1100 + VOLUME_MS));
    assert!(thrower.grenade_damage.is_empty());
    let index = volumes[0].object_index;
    // Effect: the peer starts the smoke from state 2 and receives the volume.
    // The thrower already ran its local effect and gets only the volume.
    assert_eq!(
        wire.count(&engine, OBSERVER, |p| is_state_two(p, &engine)),
        1
    );
    assert_eq!(
        wire.count(&engine, OBSERVER, |p| is_volume_creation(p, &engine)),
        1
    );
    assert_eq!(
        wire.count(&engine, THROWER, |p| is_state_two(p, &engine)),
        0
    );
    assert_eq!(
        wire.count(&engine, THROWER, |p| is_volume_creation(p, &engine)),
        1
    );
    assert_eq!(
        actor(&engine, OBSERVER).life.damage_health().unwrap().1,
        120
    );

    let profile = engine.wire_profile;
    let removal = |p: &[u8]| smoke_volume_replication::is_removal(p, profile) == Some(index);
    wire.publish(&mut engine, 1100 + VOLUME_MS - 1);
    assert_eq!(wire.count(&engine, OBSERVER, removal), 0);
    assert_eq!(actor(&engine, THROWER).smoke_volumes.volumes().len(), 1);
    wire.publish(&mut engine, 1100 + VOLUME_MS);
    assert!(actor(&engine, THROWER).smoke_volumes.volumes().is_empty());
    assert_eq!(wire.count(&engine, OBSERVER, removal), 1);
    assert_eq!(wire.count(&engine, THROWER, removal), 1);
    assert!(denials(&engine).is_empty(), "{:?}", denials(&engine));
}

#[test]
fn sabotage_frag_from_gear_four_keeps_its_detonation_path_and_stock() {
    let mut engine = sabotage_fixture();
    let launch = launch_wire(&engine, u16::from(SLOT), 4);
    let again = launch_wire(&engine, 2, 4);
    let terminal = owner_terminal(&engine, 1100);
    let mut wire = Delivered::default();
    wire.event(&mut engine, &launch, 1000);
    let thrower = actor(&engine, THROWER);
    assert_eq!(thrower.grenade_remaining, 0);
    assert_eq!(thrower.gear5_grenade_remaining, 2);
    assert_eq!(
        wire.count(&engine, OBSERVER, |p| is_creation(p, &engine)),
        1
    );
    // An empty gear refuses the next throw without touching the other gear.
    wire.event(&mut engine, &again, 1001);
    assert_eq!(denials(&engine), ["grenade_ammo_empty"]);
    assert_eq!(actor(&engine, THROWER).gear5_grenade_remaining, 2);

    // A frag detonation never creates a smoke volume and reaches every human.
    wire.event(&mut engine, &terminal, 1100);
    assert!(actor(&engine, THROWER).smoke_volumes.volumes().is_empty());
    assert_eq!(
        wire.count(&engine, OBSERVER, |p| is_state_two(p, &engine)),
        1
    );
    assert_eq!(
        wire.count(&engine, THROWER, |p| is_state_two(p, &engine)),
        1
    );
    assert_eq!(
        wire.count(&engine, OBSERVER, |p| is_volume_creation(p, &engine)),
        0
    );
}

#[test]
fn a_gear_without_a_throwable_still_refuses_the_launch() {
    let mut engine = sabotage_fixture();
    let launch = launch_wire(&engine, u16::from(SLOT), 5);
    let gear6 = launch_wire(&engine, u16::from(SLOT), 6);
    let actor = engine.active.as_mut().unwrap();
    let kit = actor.spawned_loadout.as_mut().unwrap();
    kit.inventories[0].slots[5] = None;
    // Gear 6 is not an equipment gear even when it names a throwable.
    kit.inventories[0].slots[6] = Some(SMOKE_ITEM);
    let mut wire = Delivered::default();
    wire.event(&mut engine, &launch, 1000);
    wire.event(&mut engine, &gear6, 1001);
    assert_eq!(
        denials(&engine),
        [
            "unsupported_projectile_item_or_slot",
            "unsupported_projectile_item_or_slot"
        ]
    );
    assert_eq!(
        wire.count(&engine, OBSERVER, |p| is_creation(p, &engine)),
        0
    );
}

#[test]
fn a_new_life_counts_each_equipment_gear_from_its_own_item() {
    let engine = sabotage_fixture();
    let actor = engine.active.as_ref().unwrap();
    let catalogs = resources::ThrowCatalogs {
        items: &engine.item_catalog,
        smoke: engine.smoke_grenades.as_deref(),
        gas: None,
        flash: None,
    };
    let faction = actor.reservation.character.faction;
    let mut kit = actor.equipped_loadout().unwrap();
    // Frag stock from the frag catalog, smoke stock from the smoke rules.
    assert_eq!(
        resources::initial_grenade_counts(catalogs, faction, Some(&kit)),
        [1, 2]
    );
    kit.inventories[0].slots.swap(4, 5);
    assert_eq!(
        resources::initial_grenade_counts(catalogs, faction, Some(&kit)),
        [2, 1]
    );
    kit.inventories[0].slots[4] = None;
    kit.inventories[0].slots[5] = Some(0);
    assert_eq!(
        resources::initial_grenade_counts(catalogs, faction, Some(&kit)),
        [0, 0]
    );
    assert_eq!(
        resources::initial_grenade_counts(catalogs, faction, None),
        [0, 0]
    );
}
