//! Development blast visibility over neutral triangles loaded from disk.
//! Three equal body samples approximate partial exposure. This is not native
//! Havok/material behavior, and a miss proves nothing about absent geometry.
use crate::collision::{CollisionWorld, RayHit, WorldMetadata};
use crate::replication::CharacterPosition;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct BlastRay {
    pub body_height: f64,
    pub target: [f64; 3],
    pub query_start: [f64; 3],
    pub query_end: [f64; 3],
    pub nearest: Option<RayHit>,
}
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BlastVisibility {
    NotLoaded,
    MapMismatch {
        supplied_map: String,
        expected_map: String,
    },
    QueryFailed {
        error: String,
    },
    Queried {
        metadata: WorldMetadata,
        visible_samples: u8,
        total_samples: u8,
        damage_scale: f32,
        rays: Vec<BlastRay>,
    },
}
impl BlastVisibility {
    /// Unavailable geometry/query is unknown, never an unobstructed blast.
    pub fn damage_scale(&self) -> Option<f32> {
        match self {
            Self::Queried { damage_scale, .. } => Some(*damage_scale),
            _ => None,
        }
    }
}
#[derive(Debug, Serialize)]
pub struct GrenadeBlastCheck {
    pub character_id: u32,
    pub life_generation: u32,
    pub armor_type: u8,
    pub slot: u8,
    pub generation: u32,
    pub origin: CharacterPosition,
    pub target_feet: CharacterPosition,
    pub distance: f32,
    pub hit_location: u8,
    pub damage_type: u8,
    pub unoccluded_amount_hp: f32,
    pub damage_calculation: serde_json::Value,
    pub amount_hp: f32,
    pub visibility: BlastVisibility,
}

pub fn query_blast_visibility(
    world: Option<&CollisionWorld>,
    expected_map: &str,
    blast: CharacterPosition,
    feet: CharacterPosition,
) -> BlastVisibility {
    let Some(world) = world else {
        return BlastVisibility::NotLoaded;
    };
    if world.metadata().map_name != expected_map {
        return BlastVisibility::MapMismatch {
            supplied_map: world.metadata().map_name.clone(),
            expected_map: expected_map.to_owned(),
        };
    }
    let start = [f64::from(blast.x), f64::from(blast.y), f64::from(blast.z)];
    let feet = [f64::from(feet.x), f64::from(feet.y), f64::from(feet.z)];
    if start
        .into_iter()
        .chain(feet)
        .any(|x| !x.is_finite() || x.abs() >= 2048.0)
    {
        return BlastVisibility::QueryFailed {
            error: "invalid_native_blast_position".into(),
        };
    }
    let mut rays = Vec::with_capacity(3);
    let mut visible_samples = 0;
    for body_height in [0.35, 0.9, 1.5] {
        let target = [feet[0], feet[1] + body_height, feet[2]];
        let delta = std::array::from_fn::<_, 3, _>(|i| target[i] - start[i]);
        let length = delta.iter().map(|v| v * v).sum::<f64>().sqrt();
        // Ignore only 2cm contact at either endpoint. A floor touching the
        // reported detonation must not shield every blast; intervening geometry
        // is queried double-sided. Sub-4cm samples are coincident and exposed.
        let trim = if length > 0.04 { 0.02 / length } else { 0.0 };
        let query_start = std::array::from_fn(|i| start[i] + delta[i] * trim);
        let query_end = std::array::from_fn(|i| target[i] - delta[i] * trim);
        let nearest = if length <= 0.04 {
            None
        } else {
            match world.raycast_segment(query_start, query_end) {
                Ok(query) => query.nearest,
                Err(error) => {
                    return BlastVisibility::QueryFailed {
                        error: error.to_string(),
                    };
                }
            }
        };
        visible_samples += u8::from(nearest.is_none());
        rays.push(BlastRay {
            body_height,
            target,
            query_start,
            query_end,
            nearest,
        });
    }
    BlastVisibility::Queried {
        metadata: world.metadata().clone(),
        visible_samples,
        total_samples: 3,
        damage_scale: f32::from(visible_samples) / 3.0,
        rays,
    }
}

/// Snapshot of an eligible living character. The match roster owns identity,
/// position and loadout; the client never supplies this target list.
#[derive(Clone, Copy, Debug)]
pub struct CharacterBlastTarget {
    pub character_id: u32,
    pub life_generation: u32,
    pub feet: CharacterPosition,
    pub armor: crate::armor::ArmorProfile,
}
pub const MAX_BLAST_TARGETS: usize = 128;

/// Resolve one detonation against every nearby snapshot, independently. A clear
/// sample for one actor must never authorize damage to another actor in cover.
pub fn resolve_grenade_blast(
    world: Option<&CollisionWorld>,
    expected_map: &str,
    slot: u8,
    generation: u32,
    blast: CharacterPosition,
    targets: &[CharacterBlastTarget],
) -> Result<Vec<GrenadeBlastCheck>, &'static str> {
    resolve_grenade_blast_with_catalog(
        world,
        expected_map,
        slot,
        generation,
        blast,
        targets,
        crate::item_catalog::bundled()?,
    )
}
pub fn resolve_grenade_blast_with_catalog(
    world: Option<&CollisionWorld>,
    expected_map: &str,
    slot: u8,
    generation: u32,
    blast: CharacterPosition,
    targets: &[CharacterBlastTarget],
    catalog: &crate::item_catalog::ItemCatalog,
) -> Result<Vec<GrenadeBlastCheck>, &'static str> {
    resolve_grenade_blast_for_item(
        world,
        expected_map,
        slot,
        generation,
        blast,
        targets,
        catalog.default_grenade_item()?,
        catalog,
    )
}
#[allow(clippy::too_many_arguments)]
pub fn resolve_grenade_blast_for_item(
    world: Option<&CollisionWorld>,
    expected_map: &str,
    slot: u8,
    generation: u32,
    blast: CharacterPosition,
    targets: &[CharacterBlastTarget],
    item_id: u16,
    catalog: &crate::item_catalog::ItemCatalog,
) -> Result<Vec<GrenadeBlastCheck>, &'static str> {
    if slot >= 3 || generation == 0 || targets.len() > MAX_BLAST_TARGETS {
        return Err("invalid_blast_roster");
    }
    let mut ids = std::collections::BTreeSet::new();
    for target in targets {
        if target.life_generation == 0
            || target.character_id & 0xffff0000 != 0x03800000
            || target.character_id as u16 == u16::MAX
            || !ids.insert(target.character_id)
        {
            return Err("invalid_blast_roster");
        }
    }
    let profile = crate::armor::GrenadeProfile::from_item_id_with_catalog(item_id, catalog)?;
    let mut checks = Vec::new();
    for target in targets {
        let distance = ((target.feet.x - blast.x).powi(2)
            + (target.feet.y - blast.y).powi(2)
            + (target.feet.z - blast.z).powi(2))
        .sqrt();
        let raw = profile.native_character_damage(distance)?;
        let unoccluded_amount_hp = target
            .armor
            .adjust_for_kind(raw, crate::damage::DamageKind::Explosive)?;
        if unoccluded_amount_hp <= 0.0 {
            continue;
        }
        let visibility = query_blast_visibility(world, expected_map, blast, target.feet);
        let amount_hp = unoccluded_amount_hp * visibility.damage_scale().unwrap_or(0.0);
        checks.push(GrenadeBlastCheck {
            character_id: target.character_id,
            life_generation: target.life_generation,
            armor_type: target.armor.armor_type(),
            slot,
            generation,
            origin: blast,
            target_feet: target.feet,
            distance,
            hit_location: raw.hit_location,
            damage_type: profile.native_damage_type(),
            unoccluded_amount_hp,
            damage_calculation: target
                .armor
                .calculation(raw, crate::damage::DamageKind::Explosive)?,
            amount_hp,
            visibility,
        });
    }
    Ok(checks)
}
