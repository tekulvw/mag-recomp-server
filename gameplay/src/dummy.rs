//! Server-owned stationary target; no admitted peer or stored Player mutation.

pub const OBJECT_INDEX: u16 = 1;
pub const OBJECT_ID: u32 = 0x0380_0001;
pub const SERVER_OWNER: u16 = 0x0105;

pub const INITIAL_HEALTH_QUARTER_POINTS: u16 = 400;
/// Authored Alpha on-foot alternate point65; no guessed ground-height offset.
pub fn position() -> Result<crate::replication::CharacterPosition, &'static str> {
    crate::spawn_catalog::SpawnCatalog::load(crate::spawn_catalog::default_path())
        .map_err(|_| "spawn_catalog_unavailable")?
        .alternate_position()
        .ok_or("missing_alternate_spawn")
}

/// An independent remote character and attacker Alpha squad; no GameState resend.
pub fn creation(sequence: u32, timestamp_ms: u32) -> Result<Vec<u8>, &'static str> {
    creation_with_catalog(sequence, timestamp_ms, crate::item_catalog::bundled()?)
}
pub fn creation_with_catalog(
    sequence: u32,
    timestamp_ms: u32,
    catalog: &crate::item_catalog::ItemCatalog,
) -> Result<Vec<u8>, &'static str> {
    creation_with_catalog_and_profile(
        sequence,
        timestamp_ms,
        catalog,
        crate::wire_profile::WireProfile::prototype(),
    )
}
pub fn creation_with_catalog_and_profile(
    sequence: u32,
    timestamp_ms: u32,
    catalog: &crate::item_catalog::ItemCatalog,
    profile: crate::wire_profile::WireProfile,
) -> Result<Vec<u8>, &'static str> {
    if !profile.catalog_item_ids_valid() {
        return Err("unsupported_dummy_equipment_profile");
    }
    if sequence <= 1 {
        return Err("invalid_dummy_creation_sequence");
    }
    let loadout = catalog.valor_loadout(0).ok_or("missing_dummy_loadout")?;
    creation_with_loadout(sequence, timestamp_ms, &loadout, 0, profile)
}

/// Explicitly supplied build-scoped inventory; no prototype fallback.
pub(crate) fn creation_with_loadout(
    sequence: u32,
    timestamp_ms: u32,
    loadout: &crate::replication::CharacterLoadout,
    faction: u8,
    profile: crate::wire_profile::WireProfile,
) -> Result<Vec<u8>, &'static str> {
    creation_with_named_loadout(sequence, timestamp_ms, loadout, faction, profile, "Target")
}
pub(crate) fn creation_with_named_loadout(
    sequence: u32,
    timestamp_ms: u32,
    loadout: &crate::replication::CharacterLoadout,
    faction: u8,
    profile: crate::wire_profile::WireProfile,
    name: &str,
) -> Result<Vec<u8>, &'static str> {
    use crate::replication::*;
    if sequence <= 1 {
        return Err("invalid_dummy_creation_sequence");
    }
    PlayerAndSquad {
        sequence,
        timestamp_ms,
        player: server_character(faction, name, SquadAddress::new(0, 0, 0)?),
        squad: SquadCreation {
            object_index: 0,
            owner_index: SERVER_OWNER,
            members: [
                OBJECT_INDEX,
                u16::MAX,
                u16::MAX,
                u16::MAX,
                u16::MAX,
                u16::MAX,
                u16::MAX,
                u16::MAX,
            ],
            leader_index: OBJECT_INDEX,
        },
    }
    .encode_with_inventory_with_profile(loadout, profile)
}

/// Bot Character only: authored squad objects already exist in the ready snapshot.
pub(crate) fn character_with_loadout(
    sequence: u32,
    timestamp_ms: u32,
    loadout: &crate::replication::CharacterLoadout,
    faction: u8,
    profile: crate::wire_profile::WireProfile,
    name: &str,
    squad: crate::replication::SquadAddress,
) -> Result<Vec<u8>, &'static str> {
    character_snapshot(
        OBJECT_INDEX,
        sequence,
        timestamp_ms,
        loadout,
        faction,
        profile,
        name,
        squad,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn character_snapshot(
    object_index: u16,
    sequence: u32,
    timestamp_ms: u32,
    loadout: &crate::replication::CharacterLoadout,
    faction: u8,
    profile: crate::wire_profile::WireProfile,
    name: &str,
    squad: crate::replication::SquadAddress,
    physical: Option<&crate::replication::CharacterRespawn>,
) -> Result<Vec<u8>, &'static str> {
    let mut character = server_character(faction, name, squad);
    character.object_index = object_index;
    if object_index != OBJECT_INDEX {
        character.info.account_id = 0x8000_0000 | u32::from(object_index);
    }
    crate::replication::multiplayer::RemoteCharacterSnapshot {
        sequence,
        timestamp_ms,
        character: &character,
        inventory: Some(loadout),
        physical,
    }
    .encode_with_profile(profile)
}
fn server_character(
    faction: u8,
    name: &str,
    squad: crate::replication::SquadAddress,
) -> crate::replication::CharacterCreation {
    use crate::replication::*;
    CharacterCreation {
        object_index: OBJECT_INDEX,
        owner_index: SERVER_OWNER,
        info: CharacterInfo {
            team: squad.team(),
            faction,
            character_class: 21,
            name: name.into(),
            // Native SetClientObjectID skips exactly0x105; FFFF is not safe here.
            client_index: SERVER_OWNER,
            squad_index: squad.squad_index(),
            account_id: 2,
            np_identity: [0; 36],
        },
        mag_info: MagCharacterInfo {
            nominations: 0,
            level: 1,
            leadership_level: 1,
            skill_specialty: 4,
            head_id: 0,
            voice_id: 0,
        },
    }
}

/// Coherent remote spawn fields; active/owner/info remain absent on this delta.
pub fn respawn(sequence: u32, timestamp_ms: u32) -> Result<Vec<u8>, &'static str> {
    respawn_at(sequence, timestamp_ms, position()?)
}

/// Explicit lab placement; normal typed codec validates native coordinate bounds.
pub fn respawn_at(
    sequence: u32,
    timestamp_ms: u32,
    position: crate::replication::CharacterPosition,
) -> Result<Vec<u8>, &'static str> {
    respawn_at_with_catalog(
        sequence,
        timestamp_ms,
        position,
        crate::item_catalog::bundled()?,
    )
}
pub fn respawn_at_with_catalog(
    sequence: u32,
    timestamp_ms: u32,
    position: crate::replication::CharacterPosition,
    catalog: &crate::item_catalog::ItemCatalog,
) -> Result<Vec<u8>, &'static str> {
    respawn_at_with_catalog_and_profile(
        sequence,
        timestamp_ms,
        position,
        catalog,
        crate::wire_profile::WireProfile::prototype(),
    )
}
pub fn respawn_at_with_catalog_and_profile(
    sequence: u32,
    timestamp_ms: u32,
    position: crate::replication::CharacterPosition,
    catalog: &crate::item_catalog::ItemCatalog,
    profile: crate::wire_profile::WireProfile,
) -> Result<Vec<u8>, &'static str> {
    if !profile.catalog_item_ids_valid() {
        return Err("unsupported_dummy_equipment_profile");
    }
    let loadout = catalog.valor_loadout(0).ok_or("missing_dummy_loadout")?;
    respawn_with_loadout(sequence, timestamp_ms, position, &loadout, profile)
}

pub(crate) fn respawn_with_loadout(
    sequence: u32,
    timestamp_ms: u32,
    position: crate::replication::CharacterPosition,
    loadout: &crate::replication::CharacterLoadout,
    profile: crate::wire_profile::WireProfile,
) -> Result<Vec<u8>, &'static str> {
    respawn_for(
        OBJECT_INDEX,
        sequence,
        timestamp_ms,
        position,
        loadout,
        profile,
    )
}

pub(crate) fn respawn_for(
    object_index: u16,
    sequence: u32,
    timestamp_ms: u32,
    position: crate::replication::CharacterPosition,
    loadout: &crate::replication::CharacterLoadout,
    profile: crate::wire_profile::WireProfile,
) -> Result<Vec<u8>, &'static str> {
    use crate::replication::*;
    CharacterRespawn {
        sequence,
        timestamp_ms,
        object_index,
        state: CharacterState {
            state: 1,
            stance: 0,
            user_data: [0; 3],
        },
        position,
        pitch: 127,
        yaw: 0,
        damage: CharacterDamage {
            last_location_and_type: 0,
            health_quarter_points: INITIAL_HEALTH_QUARTER_POINTS,
            conditions: 0,
            last_attacker: u32::MAX,
            last_source_object: u32::MAX,
            last_user_data: 0,
        },
    }
    .encode_with_loadout_with_profile(loadout, profile)
}
