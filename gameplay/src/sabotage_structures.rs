//! Collision that Sabotage objectives place, kept in step with objective state.
//!
//! # Behaviour
//!
//! A Sabotage objective places its own objects when it enters a state: the A
//! and B consoles (one console per state), the C server racks, and the antenna
//! raised above C. None of them is compiled static geometry. A match profile's
//! `sabotage_structures` index lists, per objective, the world-posed collision
//! packs of each authored state sequence (see docs/sabotage-structures.md):
//!
//! * every objective state names a sequence slot: a sequence index, -1 (no
//!   sequence: its objects are removed) or -2 (keep what stands);
//! * entering a state whose slot differs from the one in place removes the
//!   objects the previous sequence placed itself and places the new one's;
//! * a sequence may start fire-and-forget dependents; their objects outlive
//!   the sequence that started them and are removed only when a later
//!   dependent stops them (C's destroyed antenna replaces the intact one);
//!   a delayed creation is placed at once, in its end state;
//! * when the objective world restarts (a fresh round), everything placed is
//!   removed and placed again from the new initial states;
//! * standing packs (the fixed bodies of animated props, which the static
//!   world leaves out with their moving parts) are placed for the whole match.
//!
//! The placed packs live in a `SequenceCollisionState`, so bot movement and
//! route gating treat them like Acquisition and Domination structures. Each
//! pack keeps its collision tag, and bot movement collides with it only when
//! the character movement row collides with that layer.
use crate::collision::{CollisionLimits, CollisionWorld};
use crate::domination_structural_effects::authored_state_name;
use crate::domination_structural_world::{PlacedPack, SequenceCollisionState};
use crate::dynamic_collision::Pose;
use crate::objective_catalog::ObjectiveCatalog;
use crate::objective_control::ObjectiveWorld;
use crate::replication::ObjectiveState;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

pub const SCHEMA: &str = "mag_sabotage_objective_collision_v1";
const MAX_INDEX_BYTES: u64 = 1024 * 1024;
const MAX_OBJECTIVES: usize = 32;
const MAX_SEQUENCES: usize = 16;
const MAX_DEPENDENTS: usize = 64;
const MAX_SURFACES: usize = 32;
const STATES: [&str; 12] = [
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
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    schema: String,
    map: String,
    level_archive_sha256: String,
    level_clv_sha256: String,
    global_data_sha256: String,
    physics_sha256: String,
    objectives: Vec<ObjectiveRow>,
    dependents: BTreeMap<String, DependentRow>,
    /// Packs that stand all match, such as the fixed bodies of animated
    /// props the static world leaves out.
    #[serde(default)]
    standing: Vec<SurfaceRow>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ObjectiveRow {
    objective_token: String,
    class: String,
    definition_sha256: String,
    state_sequence_indices: BTreeMap<String, i8>,
    sequences: Vec<SequenceRow>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SequenceRow {
    source_sha256: String,
    surfaces: Vec<SurfaceRow>,
    starts: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DependentRow {
    surfaces: Vec<SurfaceRow>,
    starts: Vec<String>,
    stops: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SurfaceRow {
    object_token: String,
    collision_token: String,
    packfile_sha256: String,
    neutral_sha256: String,
    collision_tag: u8,
    material_indices: Vec<u8>,
    start_seconds: f32,
    pose: Pose,
}

#[derive(Clone)]
struct Surface {
    packfile_sha256: String,
    collision_tag: u8,
    pose: Pose,
    mesh: CollisionWorld,
}

#[derive(Clone)]
struct Sequence {
    surfaces: Vec<Surface>,
    /// Dependents this sequence starts, as dependent ordinals.
    starts: Vec<usize>,
}

#[derive(Clone)]
struct Dependent {
    surfaces: Vec<Surface>,
    starts: Vec<usize>,
    stops: Vec<usize>,
}

#[derive(Clone)]
struct Objective {
    object_index: u16,
    /// Slot per authored state name, in `STATES` order.
    slots: [i8; 12],
    sequences: Vec<Sequence>,
}

/// One map's validated Sabotage objective collision, bound to its objective
/// catalog. Shared by every profile on the map.
pub struct Assets {
    map: String,
    index_sha256: String,
    objectives: Vec<Objective>,
    dependents: Vec<Dependent>,
    standing: Vec<Surface>,
}

fn hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl Assets {
    /// Load `index.json` and the neutral pack meshes beside it, and bind every
    /// objective to `catalog` on `map`.
    pub fn load(path: &Path, map: &str, catalog: &ObjectiveCatalog) -> Result<Self, &'static str> {
        use sha2::Digest;
        let mut raw = Vec::new();
        std::fs::File::open(path)
            .map_err(|_| "sabotage_structures_index_missing")?
            .take(MAX_INDEX_BYTES + 1)
            .read_to_end(&mut raw)
            .map_err(|_| "sabotage_structures_index_invalid")?;
        if raw.len() as u64 > MAX_INDEX_BYTES {
            return Err("sabotage_structures_index_too_large");
        }
        let index: Index =
            serde_json::from_slice(&raw).map_err(|_| "sabotage_structures_index_invalid")?;
        let directory = path.parent().ok_or("sabotage_structures_index_invalid")?;
        let mut meshes = BTreeMap::new();
        let assets = Self::build(index, map, catalog, |neutral, packfile| {
            if let Some(mesh) = meshes.get(neutral) {
                return Ok(Clone::clone(mesh));
            }
            let mesh = CollisionWorld::load(
                directory.join(format!("{neutral}.json")),
                CollisionLimits::default(),
            )
            .map_err(|_| "sabotage_structures_mesh_invalid")?;
            if mesh.source_sha256() != neutral
                || !mesh
                    .metadata()
                    .provenance
                    .iter()
                    .any(|item| item == &format!("packfile_sha256={packfile}"))
            {
                return Err("sabotage_structures_mesh_source_mismatch");
            }
            meshes.insert(neutral.to_owned(), mesh.clone());
            Ok(mesh)
        })?;
        Ok(Self {
            index_sha256: format!("{:x}", sha2::Sha256::digest(&raw)),
            ..assets
        })
    }

    fn build(
        index: Index,
        map: &str,
        catalog: &ObjectiveCatalog,
        mut mesh: impl FnMut(&str, &str) -> Result<CollisionWorld, &'static str>,
    ) -> Result<Self, &'static str> {
        if index.schema != SCHEMA
            || index.map != map
            || !catalog.is_sabotage()
            || [
                &index.level_archive_sha256,
                &index.level_clv_sha256,
                &index.global_data_sha256,
                &index.physics_sha256,
            ]
            .into_iter()
            .any(|digest| !hex(digest, 64))
        {
            return Err("sabotage_structures_source_mismatch");
        }
        if index.objectives.is_empty()
            || index.objectives.len() > MAX_OBJECTIVES
            || index.dependents.len() > MAX_DEPENDENTS
        {
            return Err("sabotage_structures_count_invalid");
        }
        let ordinals: BTreeMap<String, usize> = index
            .dependents
            .keys()
            .enumerate()
            .map(|(ordinal, token)| (token.clone(), ordinal))
            .collect();
        let resolve = |tokens: &[String]| -> Result<Vec<usize>, &'static str> {
            tokens
                .iter()
                .map(|token| {
                    ordinals
                        .get(token)
                        .copied()
                        .ok_or("sabotage_structures_dependent_unbound")
                })
                .collect()
        };
        let mut surfaces = |rows: Vec<SurfaceRow>| -> Result<Vec<Surface>, &'static str> {
            if rows.len() > MAX_SURFACES {
                return Err("sabotage_structures_count_invalid");
            }
            rows.into_iter()
                .map(|row| {
                    if !hex(&row.object_token, 8)
                        || !hex(&row.collision_token, 8)
                        || row.collision_token == "00000000"
                        || !hex(&row.packfile_sha256, 64)
                        || !hex(&row.neutral_sha256, 64)
                        || row.collision_tag >= 32
                        || row.material_indices.len() > 64
                        || !row.start_seconds.is_finite()
                        || row.start_seconds < 0.0
                    {
                        return Err("sabotage_structures_surface_invalid");
                    }
                    row.pose.validate()?;
                    let mesh = mesh(&row.neutral_sha256, &row.packfile_sha256)?;
                    if mesh.metadata().map_name != map || mesh.stats().triangles == 0 {
                        return Err("sabotage_structures_mesh_source_mismatch");
                    }
                    Ok(Surface {
                        packfile_sha256: row.packfile_sha256,
                        collision_tag: row.collision_tag,
                        pose: row.pose,
                        mesh,
                    })
                })
                .collect()
        };
        let mut dependents = Vec::with_capacity(index.dependents.len());
        for row in index.dependents.into_values() {
            dependents.push(Dependent {
                starts: resolve(&row.starts)?,
                stops: resolve(&row.stops)?,
                surfaces: surfaces(row.surfaces)?,
            });
        }
        let standing = surfaces(index.standing)?;
        let mut objectives = Vec::with_capacity(index.objectives.len());
        let mut bound = BTreeSet::new();
        for row in index.objectives {
            let token = row
                .objective_token
                .strip_prefix("0x")
                .filter(|token| hex(token, 8))
                .and_then(|token| u32::from_str_radix(token, 16).ok())
                .ok_or("sabotage_structures_objective_invalid")?;
            let (object_index, _) = catalog
                .binding(token)
                .ok_or("sabotage_structures_objective_unbound")?;
            if !matches!(
                row.class.as_str(),
                "ObjectiveSecureAndHoldProperties" | "ObjectiveDemolitionStructuralProperties"
            ) || !hex(&row.definition_sha256, 64)
                || row.sequences.is_empty()
                || row.sequences.len() > MAX_SEQUENCES
                || row.state_sequence_indices.len() != STATES.len()
                || object_index == STANDING
                || !bound.insert(object_index)
            {
                return Err("sabotage_structures_objective_invalid");
            }
            let mut slots = [0; 12];
            for (slot, state) in slots.iter_mut().zip(STATES) {
                let index = *row
                    .state_sequence_indices
                    .get(state)
                    .ok_or("sabotage_structures_objective_invalid")?;
                if index < -2 || index >= 0 && index as usize >= row.sequences.len() {
                    return Err("sabotage_structures_objective_invalid");
                }
                *slot = index;
            }
            let mut sequences = Vec::with_capacity(row.sequences.len());
            for sequence in row.sequences {
                if !hex(&sequence.source_sha256, 64) {
                    return Err("sabotage_structures_objective_invalid");
                }
                sequences.push(Sequence {
                    starts: resolve(&sequence.starts)?,
                    surfaces: surfaces(sequence.surfaces)?,
                });
            }
            objectives.push(Objective {
                object_index,
                slots,
                sequences,
            });
        }
        Ok(Self {
            map: map.to_owned(),
            index_sha256: String::new(),
            objectives,
            dependents,
            standing,
        })
    }

    pub fn map(&self) -> &str {
        &self.map
    }

    pub fn index_sha256(&self) -> &str {
        &self.index_sha256
    }

    pub fn objective_count(&self) -> usize {
        self.objectives.len()
    }
}

/// One placement change, for the event record.
#[derive(Debug, PartialEq)]
pub(crate) struct Change {
    pub(crate) object_index: u16,
    pub(crate) state: u8,
    pub(crate) sequence: i8,
    pub(crate) packs: usize,
}

/// The objectives' placed collision in the current match.
pub(crate) struct Runtime {
    assets: Arc<Assets>,
    /// Character movement row the placed packs are filtered by.
    movement_row: u32,
    /// Sequence slot in place per objective; None before the first sync.
    current: Vec<Option<i8>>,
    /// Whether the objective world was pristine at the last sync.
    pristine: bool,
    collision: SequenceCollisionState,
}

impl Runtime {
    pub(crate) fn new(assets: Arc<Assets>, movement_row: u32) -> Self {
        let mut collision = SequenceCollisionState::default();
        collision.set_placed_packs(STANDING, 0, placed(&assets.standing));
        Self {
            current: vec![None; assets.objectives.len()],
            assets,
            movement_row,
            pristine: false,
            collision,
        }
    }

    pub(crate) fn collision(&self) -> (&SequenceCollisionState, u32) {
        (&self.collision, self.movement_row)
    }

    /// Bring the placed collision in step with `world`'s objective states.
    pub(crate) fn sync(&mut self, world: &ObjectiveWorld) -> Result<Vec<Change>, &'static str> {
        self.sync_states(world.is_pristine(), |index| world.state(index))
    }

    /// `sync` over a pristine flag and each bound objective's current state.
    fn sync_states(
        &mut self,
        pristine: bool,
        state_of: impl Fn(u16) -> Option<ObjectiveState>,
    ) -> Result<Vec<Change>, &'static str> {
        let restarted = pristine && !self.pristine;
        self.pristine = pristine;
        if restarted {
            for (row, current) in self.assets.objectives.iter().zip(&mut self.current) {
                if current.take().is_some() {
                    for slot in 0..=self.assets.dependents.len() {
                        self.collision
                            .set_placed_packs(row.object_index, slot as u8, Vec::new());
                    }
                }
            }
        }
        let assets = Arc::clone(&self.assets);
        let mut changes = Vec::new();
        for (row, current) in assets.objectives.iter().zip(&mut self.current) {
            let state =
                state_of(row.object_index).ok_or("sabotage_structures_objective_missing")?;
            let Some(name) = authored_state_name(state) else {
                continue;
            };
            let slot = row.slots[STATES.iter().position(|s| *s == name).expect("state")];
            if slot == -2 || *current == Some(slot) {
                continue;
            }
            *current = Some(slot);
            let sequence = usize::try_from(slot).ok().map(|i| &row.sequences[i]);
            let own = sequence.map_or_else(Vec::new, |s| placed(&s.surfaces));
            let mut packs = own.len();
            self.collision.set_placed_packs(row.object_index, 0, own);
            let mut pending: Vec<usize> = sequence.map_or_else(Vec::new, |s| s.starts.clone());
            let mut started = BTreeSet::new();
            while let Some(ordinal) = pending.pop() {
                if !started.insert(ordinal) {
                    continue;
                }
                let dependent = &assets.dependents[ordinal];
                for &stop in &dependent.stops {
                    self.collision
                        .set_placed_packs(row.object_index, slot_of(stop), Vec::new());
                }
                packs += dependent.surfaces.len();
                self.collision.set_placed_packs(
                    row.object_index,
                    slot_of(ordinal),
                    placed(&dependent.surfaces),
                );
                pending.extend(&dependent.starts);
            }
            changes.push(Change {
                object_index: row.object_index,
                state: state as u8,
                sequence: slot,
                packs,
            });
        }
        Ok(changes)
    }
}

/// Objective key of the standing packs; no catalog object index reaches it.
const STANDING: u16 = u16::MAX;

/// Slot of a dependent's packs under its starting objective; zero holds the
/// objective's own sequence.
fn slot_of(ordinal: usize) -> u8 {
    u8::try_from(ordinal + 1).expect("bounded dependents")
}

fn placed(surfaces: &[Surface]) -> Vec<PlacedPack> {
    surfaces
        .iter()
        .map(|surface| PlacedPack {
            pose: surface.pose,
            mesh: surface.mesh.clone(),
            collision_tag: surface.collision_tag,
            packfile_sha256: surface.packfile_sha256.clone(),
        })
        .collect()
}

#[cfg(test)]
#[path = "sabotage_structures_tests.rs"]
mod tests;
