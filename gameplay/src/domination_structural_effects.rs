//! Original Structural sequence inputs and detached playback planning.
use crate::{dynamic_collision::Pose, replication::ObjectiveState};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    schema: String,
    pub map: String,
    pub metadata_sha256: String,
    pub level_archive_sha256: String,
    pub level_clv_sha256: String,
    pub global_data_sha256: String,
    pub physics_sha256: String,
    collision_resources: BTreeMap<String, CollisionResource>,
    unresolved_collision_tokens: BTreeSet<String>,
    #[serde(default)]
    confirmed_absent_collision_tokens: BTreeSet<String>,
    #[serde(default)]
    collision_lookup_archives: Vec<String>,
    #[serde(default)]
    material_penetration_costs: BTreeMap<u8, f32>,
    definitions: BTreeMap<String, Definition>,
    placements: Vec<Placement>,
    sequences: BTreeMap<String, Sequence>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CollisionResource {
    packfile_sha256: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Placement {
    objective_token: String,
    definition_token: String,
    pose: Pose,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Definition {
    state_sequence_indices: BTreeMap<String, i8>,
    sequence_tokens: Vec<String>,
    blast_properties: Option<BlastProperties>,
    definition_sha256: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "PascalCase")]
pub struct BlastProperties {
    #[serde(rename = "SubType")]
    pub subtype: u8,
    pub full_damage_radius: f32,
    pub half_damage_radius: f32,
    pub damage_value: f32,
    pub material_penetration: f32,
    pub character_damage_over_time_radius: f32,
    pub character_damage_over_time_value: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InstantaneousBlast {
    pub origins: Vec<[f32; 3]>,
    pub inner_radius: f32,
    pub outer_radius: f32,
    pub damage: f32,
    pub material_penetration: f32,
}

impl BlastProperties {
    /// The server supplies current transformed objective and trigger positions.
    /// A missing selected trigger uses the original objective-position fallback.
    pub fn instantaneous_blast(
        self,
        objective_position: [f32; 3],
        selected_trigger_position: Option<[f32; 3]>,
    ) -> Result<InstantaneousBlast, &'static str> {
        let mut origins = vec![selected_trigger_position.unwrap_or(objective_position)];
        if self.subtype == 14 && selected_trigger_position.is_some() {
            origins.push(objective_position);
        }
        for point in &mut origins {
            point[1] += 1.0;
            if !point.iter().all(|value| value.is_finite()) {
                return Err("invalid_domination_structural_blast_origin");
            }
        }
        Ok(InstantaneousBlast {
            origins,
            inner_radius: self.full_damage_radius.min(self.half_damage_radius),
            outer_radius: self.full_damage_radius.max(self.half_damage_radius),
            damage: self.damage_value,
            material_penetration: self.material_penetration,
        })
    }
}

impl InstantaneousBlast {
    /// Unoccluded scale at a qualified target point. Material tracing and target
    /// damage admission are performed by the effects provider.
    pub fn scale(&self, distance: f32) -> Result<f32, &'static str> {
        if !distance.is_finite() || distance < 0.0 {
            return Err("invalid_domination_structural_blast_distance");
        }
        if self.outer_radius == 0.0 || distance > self.outer_radius {
            return Ok(0.0);
        }
        if self.inner_radius > 0.0 && distance <= self.inner_radius {
            return Ok((-0.5_f32).mul_add(distance / self.inner_radius, 1.0));
        }
        let width = self.outer_radius - self.inner_radius;
        if distance >= self.outer_radius {
            return Ok(0.0);
        }
        let fraction = if width >= 0.00001 {
            (distance - self.inner_radius) / width
        } else {
            0.0
        };
        Ok((-0.5_f32).mul_add(fraction, 0.5))
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Sequence {
    source_sha256: String,
    events: Vec<Event>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum Event {
    #[serde(rename = "create_animated_object")]
    CreateAnimatedObject(Creation),
    VisualSequenceControl {
        event_index: u8,
    },
    PrintEventDef {
        event_index: u8,
    },
    ParticleEventDef {
        event_index: u8,
    },
    SoundEventDef {
        event_index: u8,
    },
    ExplosionEventDef {
        event_index: u8,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Creation {
    pub event_index: u8,
    pub object_token: String,
    pub geometry_token: String,
    pub collision_token: String,
    pub skeleton_token: String,
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub start_seconds: f32,
    pub duration_seconds: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CollisionResourceRef<'a> {
    None,
    Bound(&'a [String]),
    ConfirmedAbsent,
    Unresolved,
}

#[derive(Debug, PartialEq)]
pub struct PlaybackChange<'a> {
    pub stops: Vec<&'a Creation>,
    pub starts: Vec<&'a Creation>,
}

#[derive(Debug, PartialEq)]
pub struct SequencePlan<'a> {
    /// Signed state index passed to native SetSequenceIndex after state entry.
    pub replicated_index: Option<i8>,
    /// None retains playback. Some stops the prior sequence, then starts the new one.
    pub change: Option<PlaybackChange<'a>>,
}

const STATE_NAMES: [&str; 12] = [
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

impl Catalog {
    pub fn from_json(json: &str) -> Result<Self, &'static str> {
        if json.len() > 256 * 1024 {
            return Err("domination_structural_effects_catalog_too_large");
        }
        let catalog: Self =
            serde_json::from_str(json).map_err(|_| "invalid_domination_structural_effects_json")?;
        catalog.validate()?;
        Ok(catalog)
    }

    fn validate(&self) -> Result<(), &'static str> {
        if self.schema != "mag_domination_structural_effect_inputs_v1"
            || self.map.is_empty()
            || !self
                .map
                .bytes()
                .all(|b| b == b'_' || b.is_ascii_lowercase() || b.is_ascii_digit())
            || [
                &self.metadata_sha256,
                &self.level_archive_sha256,
                &self.level_clv_sha256,
                &self.global_data_sha256,
                &self.physics_sha256,
            ]
            .into_iter()
            .any(|value| !hex(value, 64))
            || self.placements.is_empty()
            || self.placements.len() > 128
        {
            return Err("invalid_domination_structural_effects_identity");
        }
        let mut objective_tokens = BTreeSet::new();
        for placement in &self.placements {
            placement.pose.validate()?;
            let Some(token) = placement.objective_token.strip_prefix("0x") else {
                return Err("invalid_domination_structural_objective_token");
            };
            if !hex(token, 8)
                || !self.definitions.contains_key(&placement.definition_token)
                || !objective_tokens.insert(token)
            {
                return Err("invalid_domination_structural_placement");
            }
        }
        for (key, definition) in &self.definitions {
            if !key.strip_prefix("0x").is_some_and(|token| hex(token, 8))
                || !hex(&definition.definition_sha256, 64)
                || definition.state_sequence_indices.len() != STATE_NAMES.len()
                || STATE_NAMES.iter().any(|state| {
                    definition
                        .state_sequence_indices
                        .get(*state)
                        .is_none_or(|&index| {
                            index < -2
                                || index >= 0 && index as usize >= definition.sequence_tokens.len()
                        })
                })
                || definition
                    .sequence_tokens
                    .iter()
                    .any(|token| !self.sequences.contains_key(token))
                || definition.blast_properties.is_some_and(|blast| {
                    [
                        blast.full_damage_radius,
                        blast.half_damage_radius,
                        blast.damage_value,
                        blast.material_penetration,
                        blast.character_damage_over_time_radius,
                        blast.character_damage_over_time_value,
                    ]
                    .into_iter()
                    .any(|value| !value.is_finite() || value < 0.0)
                })
            {
                return Err("invalid_domination_structural_definition");
            }
        }
        let mut collision_tokens = BTreeSet::new();
        for (token, sequence) in &self.sequences {
            if !hex(token, 8) || !hex(&sequence.source_sha256, 64) || sequence.events.len() > 128 {
                return Err("invalid_domination_structural_sequence");
            }
            for (offset, event) in sequence.events.iter().enumerate() {
                let index = match event {
                    Event::VisualSequenceControl { event_index }
                    | Event::PrintEventDef { event_index }
                    | Event::ParticleEventDef { event_index }
                    | Event::SoundEventDef { event_index }
                    | Event::ExplosionEventDef { event_index } => *event_index,
                    Event::CreateAnimatedObject(creation) => {
                        if [
                            &creation.object_token,
                            &creation.geometry_token,
                            &creation.collision_token,
                            &creation.skeleton_token,
                        ]
                        .into_iter()
                        .any(|token| !hex(token, 8))
                            || creation.start_seconds != 0.0
                            || creation.duration_seconds != -1.0
                            || creation
                                .position
                                .into_iter()
                                .chain(creation.rotation)
                                .any(|n| !n.is_finite())
                        {
                            return Err("invalid_domination_structural_creation_event");
                        }
                        if creation.collision_token != "00000000" {
                            collision_tokens.insert(creation.collision_token.as_str());
                        }
                        creation.event_index
                    }
                };
                if usize::from(index) != offset {
                    return Err("invalid_domination_structural_event_order");
                }
            }
        }
        let mut qualified = BTreeSet::new();
        for (token, resource) in &self.collision_resources {
            if resource.packfile_sha256.is_empty()
                || resource
                    .packfile_sha256
                    .iter()
                    .any(|digest| !hex(digest, 64))
            {
                return Err("invalid_domination_structural_collision_resource");
            }
            qualified.insert(token.as_str());
        }
        for token in &self.unresolved_collision_tokens {
            if !qualified.insert(token.as_str()) {
                return Err("ambiguous_domination_structural_collision_resource");
            }
        }
        const ORIGINAL_SHARED: [&str; 2] = [
            "a2e8345cd9f5c650ac22544096ba83e8c9e6f0fffd0ef69eb8791fb265f9baad",
            "b49c9435be729832029ad7019874d478080dc9764ebde1809b2851d353bc769f",
        ];
        if !self.confirmed_absent_collision_tokens.is_empty()
            && self.collision_lookup_archives != ORIGINAL_SHARED
        {
            return Err("unproved_domination_structural_collision_absence");
        }
        if !self.material_penetration_costs.is_empty()
            && (self.collision_lookup_archives != ORIGINAL_SHARED
                || !self
                    .material_penetration_costs
                    .keys()
                    .copied()
                    .eq((0..27).chain([255]))
                || self
                    .material_penetration_costs
                    .values()
                    .any(|&cost| !cost.is_finite() || (cost < 0.0 && cost != -1.0)))
        {
            return Err("invalid_domination_material_penetration_costs");
        }
        for token in &self.confirmed_absent_collision_tokens {
            if !qualified.insert(token.as_str()) {
                return Err("ambiguous_domination_structural_collision_resource");
            }
        }
        if qualified != collision_tokens {
            return Err("incomplete_domination_structural_collision_references");
        }
        Ok(())
    }

    fn placement(&self, objective_token: u32) -> Result<&Placement, &'static str> {
        let key = format!("0x{objective_token:08x}");
        self.placements
            .iter()
            .find(|placement| placement.objective_token == key)
            .ok_or("unbound_domination_structural_effect_objective")
    }

    fn definition(&self, objective_token: u32) -> Result<&Definition, &'static str> {
        Ok(&self.definitions[&self.placement(objective_token)?.definition_token])
    }

    pub(crate) fn placed_objectives(&self) -> impl Iterator<Item = u32> + '_ {
        self.placements.iter().map(|row| {
            u32::from_str_radix(&row.objective_token[2..], 16).expect("validated objective token")
        })
    }

    pub(crate) fn source_is(&self, map: &str, metadata: &str, archive: &str, clv: &str) -> bool {
        self.map == map
            && self.metadata_sha256 == metadata
            && self.level_archive_sha256 == archive
            && self.level_clv_sha256 == clv
    }

    pub fn objective_pose(&self, objective_token: u32) -> Result<Pose, &'static str> {
        Ok(self.placement(objective_token)?.pose)
    }

    pub fn material_penetration_costs(&self) -> &BTreeMap<u8, f32> {
        &self.material_penetration_costs
    }

    pub fn blast_properties(&self, objective_token: u32) -> Result<BlastProperties, &'static str> {
        self.definition(objective_token)?
            .blast_properties
            .ok_or("unbound_domination_structural_blast_properties")
    }

    pub(crate) fn bound_collision_count(&self) -> usize {
        self.collision_resources.len()
    }

    pub(crate) fn pack_hashes(&self, token: &str) -> Option<&[String]> {
        Some(&self.collision_resources.get(token)?.packfile_sha256)
    }

    pub(crate) fn confirmed_absent_collisions(&self) -> &BTreeSet<String> {
        &self.confirmed_absent_collision_tokens
    }

    pub fn collision_reference(&self, creation: &Creation) -> CollisionResourceRef<'_> {
        if creation.collision_token == "00000000" {
            CollisionResourceRef::None
        } else if let Some(row) = self.collision_resources.get(&creation.collision_token) {
            CollisionResourceRef::Bound(&row.packfile_sha256)
        } else if self
            .confirmed_absent_collision_tokens
            .contains(&creation.collision_token)
        {
            CollisionResourceRef::ConfirmedAbsent
        } else {
            CollisionResourceRef::Unresolved
        }
    }

    /// Plan the OnFinishTransition sequence publication and gameplay collision changes.
    /// Stop calls traverse active events in order. The prior sequence's objects
    /// are stopped before starts install the replacement.
    pub fn sequence(
        &self,
        objective_token: u32,
        state: ObjectiveState,
        current_index: i8,
    ) -> Result<SequencePlan<'_>, &'static str> {
        use ObjectiveState::*;
        let state_name = match state {
            Inactive => "Inactive",
            Deactivated => "Deactivated",
            Activating => "Activating",
            Activated => "Activated",
            Deactivating => "Deactivating",
            Completed => "Completed",
            Repairing => "Repairing",
            Repaired => "Repaired",
            Locked => "Locked",
            Retaking => "Retaking",
            Retaken => "Retaken",
            CancelRetaken => "CancelRetaken",
            Failed => return Err("unsupported_domination_structural_sequence_state"),
        };
        let definition = self.definition(objective_token)?;
        let target = definition.state_sequence_indices[state_name];
        if target == -2 {
            return Ok(SequencePlan {
                replicated_index: None,
                change: None,
            });
        }
        let active = |index: i8| -> Result<Vec<&Creation>, &'static str> {
            if index == -1 {
                return Ok(Vec::new());
            }
            let token = definition
                .sequence_tokens
                .get(index as usize)
                .ok_or("invalid_domination_structural_sequence_slot")?;
            Ok(self.sequences[token]
                .events
                .iter()
                .filter_map(|event| match event {
                    Event::CreateAnimatedObject(creation) => Some(creation),
                    _ => None,
                })
                .collect())
        };
        let change = if current_index == target {
            None
        } else {
            Some(PlaybackChange {
                stops: active(current_index)?,
                starts: active(target)?,
            })
        };
        Ok(SequencePlan {
            replicated_index: Some(target),
            change,
        })
    }
}

/// A native level's resident terrain masks reduced to selected material slots
/// in original cell and pixel order. Source terrain assets remain private.
pub struct TerrainMaterialGrid {
    columns: usize,
    cell_size: f32,
    tile_width: usize,
    pixels: Box<[u8]>,
}

impl TerrainMaterialGrid {
    pub fn load(
        index: &std::path::Path,
        pixels: &std::path::Path,
        catalog: &Catalog,
        retail_elf_sha256: &str,
    ) -> Result<Self, &'static str> {
        use sha2::Digest;
        use std::io::Read;

        #[derive(Deserialize)]
        struct Index {
            schema: String,
            map: String,
            level_archive_sha256: String,
            level_clv_sha256: String,
            retail_elf_sha256: String,
            columns: usize,
            cell_size: f32,
            tile_width: usize,
            materials_sha256: String,
            byte_count: usize,
            source_tile_sha256: Vec<String>,
        }

        let mut source = String::new();
        std::fs::File::open(index)
            .and_then(|file| file.take(65_537).read_to_string(&mut source))
            .map_err(|_| "invalid_domination_terrain_material_index")?;
        if source.len() > 65_536 {
            return Err("invalid_domination_terrain_material_index");
        }
        let index: Index = serde_json::from_str(&source)
            .map_err(|_| "invalid_domination_terrain_material_index")?;
        const BYTES: usize = 4096 * 4096;
        if index.schema != "mag_domination_terrain_material_grid_v1"
            || index.map != catalog.map
            || index.level_archive_sha256 != catalog.level_archive_sha256
            || index.level_clv_sha256 != catalog.level_clv_sha256
            || !hex(retail_elf_sha256, 64)
            || index.retail_elf_sha256 != retail_elf_sha256
            || !matches!(index.columns, 8 | 16)
            || index.cell_size != 1024.0 / index.columns as f32
            || index.tile_width != 4096 / index.columns
            || index.byte_count != BYTES
            || !hex(&index.materials_sha256, 64)
            || index.source_tile_sha256.len() != index.columns * index.columns
            || index
                .source_tile_sha256
                .iter()
                .any(|digest| !hex(digest, 64))
        {
            return Err("domination_terrain_material_source_mismatch");
        }
        let mut buffer = Vec::with_capacity(BYTES);
        std::fs::File::open(pixels)
            .and_then(|file| file.take((BYTES + 1) as u64).read_to_end(&mut buffer))
            .map_err(|_| "invalid_domination_terrain_material_pixels")?;
        if buffer.len() != BYTES
            || format!("{:x}", sha2::Sha256::digest(&buffer)) != index.materials_sha256
            || buffer.iter().any(|&slot| slot > 26 && slot != 255)
        {
            return Err("invalid_domination_terrain_material_pixels");
        }
        Ok(Self {
            columns: index.columns,
            cell_size: index.cell_size,
            tile_width: index.tile_width,
            pixels: buffer.into_boxed_slice(),
        })
    }

    pub(crate) fn material_slots(&self) -> BTreeSet<u8> {
        self.pixels.iter().copied().collect()
    }

    pub fn sample(&self, x: f32, z: f32) -> Result<u8, &'static str> {
        if ![x, z].into_iter().all(|v| (0.0..=1024.0).contains(&v)) {
            return Err("domination_terrain_material_position_out_of_bounds");
        }
        let cell = |value: f32| (value.min(1023.0) / self.cell_size) as usize;
        let pixel = |value: f32| {
            ((value % self.cell_size) / self.cell_size * (self.tile_width - 1) as f32) as usize
        };
        let tile = cell(x) * self.columns + cell(z);
        let u = pixel(x);
        let v = self.tile_width - 1 - pixel(z);
        Ok(self.pixels[tile * self.tile_width.pow(2) + v * self.tile_width + u])
    }
}

fn hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn profile() -> Value {
        let creation = |event_index, object_token, collision_token| {
            json!({
                "kind":"create_animated_object", "event_index":event_index,
                "object_token":object_token, "collision_token":collision_token,
                "geometry_token":"00000005", "skeleton_token":"00000000",
                "position":[0.,0.,0.], "rotation":[0.,0.,0.],
                "start_seconds":0., "duration_seconds":-1.
            })
        };
        let mut indices = BTreeMap::from_iter(STATE_NAMES.into_iter().map(|name| (name, -2)));
        indices.insert("Inactive", -1);
        indices.insert("Deactivated", 0);
        indices.insert("Completed", 1);
        indices.insert("Repaired", 0);
        let digest = "a".repeat(64);
        json!({
            "schema":"mag_domination_structural_effect_inputs_v1", "map":"synthetic",
            "metadata_sha256":digest, "level_archive_sha256":digest, "level_clv_sha256":digest,
            "global_data_sha256":digest, "physics_sha256":digest,
            "collision_resources":{"00000009":{"packfile_sha256":[digest]}},
            "unresolved_collision_tokens":["00000008"],
            "definitions":{"0x00000010":{
                "state_sequence_indices":indices, "sequence_tokens":["00000022","00000023"],
                "definition_sha256":digest,
                "blast_properties":{"SubType":14,"FullDamageRadius":11.,"HalfDamageRadius":2.,"DamageValue":3.,
                    "MaterialPenetration":1.,"CharacterDamageOverTimeRadius":0.,"CharacterDamageOverTimeValue":0.}
            }},
            "placements":[{"objective_token":"0x00000011","definition_token":"0x00000010","pose":{"translation":[5,6,7],"rotation":[[1,0,0],[0,1,0],[0,0,1]]}}],
            "sequences":{
                "00000022":{"source_sha256":digest,"events":[
                    {"kind":"PrintEventDef","event_index":0},
                    creation(1,"00000006","00000009"),
                    creation(2,"00000007","00000008")
                ]},
                "00000023":{"source_sha256":digest,"events":[creation(0,"00000007","00000000")]}
            }
        })
    }

    #[test]
    fn terrain_material_sample_preserves_cell_order_and_flips_texture_v() {
        let grid = TerrainMaterialGrid {
            columns: 2,
            cell_size: 512.,
            tile_width: 4,
            pixels: (0..64).collect::<Vec<_>>().into_boxed_slice(),
        };
        assert_eq!(grid.sample(0., 0.), Ok(12));
        assert_eq!(grid.sample(512., 0.), Ok(44));
        assert_eq!(grid.sample(0., 512.), Ok(28));
        assert_eq!(grid.sample(200., 200.), Ok(9));
        assert_eq!(grid.sample(1024., 1024.), Ok(60));
        assert!(grid.sample(f32::NAN, 0.).is_err());
        assert!(grid.sample(-1., 0.).is_err());
    }

    #[test]
    fn distinguishes_keep_stop_restart_and_repeated_index() {
        use ObjectiveState::{Activating, Completed, Inactive, Repaired};
        let catalog = Catalog::from_json(&profile().to_string()).unwrap();
        let keep = catalog.sequence(0x11, Activating, 0).unwrap();
        assert_eq!(
            keep,
            SequencePlan {
                replicated_index: None,
                change: None
            }
        );
        let refresh = catalog.sequence(0x11, Repaired, 0).unwrap();
        assert_eq!(
            refresh,
            SequencePlan {
                replicated_index: Some(0),
                change: None
            }
        );
        let clear = catalog.sequence(0x11, Inactive, 0).unwrap();
        assert_eq!(clear.replicated_index, Some(-1));
        assert!(clear.change.unwrap().starts.is_empty());
        let replace = catalog.sequence(0x11, Completed, 0).unwrap();
        assert_eq!(replace.replicated_index, Some(1));
        let change = replace.change.unwrap();
        assert_eq!(
            change
                .stops
                .iter()
                .map(|evt| evt.event_index)
                .collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(change.starts.len(), 1);
        assert!(matches!(
            catalog.collision_reference(change.stops[0]),
            CollisionResourceRef::Bound(_)
        ));
        assert_eq!(
            catalog.collision_reference(change.stops[1]),
            CollisionResourceRef::Unresolved
        );
        assert_eq!(
            catalog.collision_reference(change.starts[0]),
            CollisionResourceRef::None
        );
    }

    #[test]
    fn blast_uses_trigger_then_objective_for_combined_center_subtype() {
        let catalog = Catalog::from_json(&profile().to_string()).unwrap();
        let properties = catalog.blast_properties(0x11).unwrap();
        let center = [4., 2., 1.];
        let selected = [8., 3., 4.];
        let blast = properties
            .instantaneous_blast(center, Some(selected))
            .unwrap();
        assert_eq!(blast.origins, [[8., 4., 4.], [4., 3., 1.]]);
        assert_eq!((blast.inner_radius, blast.outer_radius), (2., 11.));
        assert_eq!(blast.scale(0.), Ok(1.));
        assert_eq!(blast.scale(2.), Ok(0.5));
        assert_eq!(blast.scale(6.5), Ok(0.25));
        assert_eq!(blast.scale(11.), Ok(0.));
        assert_eq!(blast.scale(11.1), Ok(0.));
        assert!(blast.scale(f32::INFINITY).is_err());
        let fallback = properties.instantaneous_blast(center, None).unwrap();
        assert_eq!(fallback.origins, [[4., 3., 1.]]);
    }

    #[test]
    fn rejects_incomplete_resource_inventory_and_noninstantaneous_creation() {
        let mut incomplete = profile();
        incomplete["unresolved_collision_tokens"] = json!([]);
        assert!(Catalog::from_json(&incomplete.to_string()).is_err());
        let mut delayed = profile();
        delayed["sequences"]["00000022"]["events"][2]["start_seconds"] = json!(0.125);
        assert!(Catalog::from_json(&delayed.to_string()).is_err());
    }
}
