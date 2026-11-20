//! Explicit external inputs for the retail command effect runtime.

use crate::{
    active_abilities::{Ability, Rules, Side},
    active_ability_effects::EffectRules,
    active_ability_engine::{AntiAir, Boundary, SensorObjective, Setup},
    collision::CollisionWorld,
    objective_catalog::{NativeClass, Objective, ObjectiveCatalog},
    spawn_catalog::SpawnCatalog,
    structure_damage_world::Rule,
};
use serde::{Deserialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

const MAX_CONFIG_SIZE: u64 = 256 * 1024;
const RETAIL_ELF_SHA256: &str = "0089f0d6d17e48533ce7ff8d943dddd03487b925e7b04a1a055bfb907568694c";

/// Resolve the required map sidecar for a source gameplay provider. The
/// manifest can be relocated through an explicit absolute path.
pub fn manifest_path(
    explicit: Option<PathBuf>,
    provider: Option<&Path>,
    map_name: &str,
) -> io::Result<Option<PathBuf>> {
    if explicit.is_some() {
        return Ok(explicit);
    }
    let Some(provider) = provider else {
        return Ok(None);
    };
    if map_name.is_empty()
        || !map_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(invalid("invalid_active_ability_sidecar_map_name"));
    }
    Ok(Some(provider.with_file_name(format!(
        "{map_name}--active-abilities.json"
    ))))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: String,
    rules: PathBuf,
    effects: PathBuf,
    map_bindings: PathBuf,
    static_ray_receipt: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceMember {
    member: String,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EffectMembers {
    cluster: SourceMember,
    sensor: SourceMember,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Faction {
    Green,
    Black,
    Red,
}

impl Faction {
    fn wire_id(self) -> u8 {
        match self {
            Self::Green => 0,
            Self::Black => 1,
            Self::Red => 2,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EffectRow {
    side: Side,
    faction: Faction,
    rules: EffectRules,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Effects {
    schema_version: u32,
    application_id: u32,
    source_archive_sha256: String,
    source_members: EffectMembers,
    rows: Vec<EffectRow>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MapSource {
    retail_elf_sha256: String,
    level_archive_sha256: String,
    level_clv_sha256: String,
    terrain_resource_bank_sha256: String,
    gameplay_objectives_sha256: String,
    qualified_objective_poses_sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MapBindings {
    schema_version: u32,
    application_id: u32,
    map_name: String,
    source: MapSource,
    boundary: Boundary,
    vertical_bounds: [f32; 2],
    aaa: Vec<AntiAir>,
    objectives: Vec<SensorObjective>,
    damage_triggers: Vec<Rule>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FilterCount {
    root_filter_info: u32,
    child_filter_info: u32,
    mesh_count: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StaticRayReceipt {
    schema_version: u32,
    map_name: String,
    source_archive_sha256: String,
    source_clv_sha256: String,
    source_elf_sha256: String,
    source_world_sha256: String,
    source_assembly_sha256: String,
    source_root_inventory_sha256: String,
    world_sha256: String,
    world_y_bounds: [f32; 2],
    root_count: usize,
    source_mesh_count: usize,
    output_mesh_count: usize,
    reference_parent_root_count: usize,
    source_mesh_filter_counts: Vec<FilterCount>,
}

fn read_bounded(path: &Path) -> io::Result<Vec<u8>> {
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "active ability configuration requires an absolute path",
        ));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|error| io::Error::new(error.kind(), format!("{}: {error}", path.display())))?
        .take(MAX_CONFIG_SIZE + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CONFIG_SIZE {
        return Err(invalid("active_ability_input_too_large"));
    }
    Ok(bytes)
}

fn read<T: DeserializeOwned>(path: &Path) -> io::Result<T> {
    serde_json::from_slice(&read_bounded(path)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn decode_map_bindings(bytes: &[u8]) -> io::Result<MapBindings> {
    const REPROJECT: &str =
        "active_ability_map_bindings_require_reprojection (schema 2 with damage_triggers required)";
    let map: MapBindings = serde_json::from_slice(bytes).map_err(|error| {
        io::Error::new(io::ErrorKind::InvalidData, format!("{REPROJECT}: {error}"))
    })?;
    if map.schema_version != 2 {
        return Err(invalid(REPROJECT));
    }
    Ok(map)
}

fn invalid(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

fn hash_ok(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_sources(
    rules: &Rules,
    effects: &Effects,
    map: &MapBindings,
    scene: &StaticRayReceipt,
) -> io::Result<()> {
    if effects.schema_version != 1
        || effects.application_id != 20231
        || effects.source_archive_sha256 != rules.source_archive_sha256
    {
        return Err(invalid("active_ability_effect_source_mismatch"));
    }
    for (ability, source) in [
        (Ability::ClusterBomb, &effects.source_members.cluster),
        (Ability::Sensor, &effects.source_members.sensor),
    ] {
        let rule = rules
            .property(ability)
            .ok_or_else(|| invalid("active_ability_command_rule_missing"))?;
        if source.member.strip_prefix("/run/mag/ps3/") != Some(&rule.source_member)
            || source.sha256 != rule.source_sha256
        {
            return Err(invalid("active_ability_effect_member_mismatch"));
        }
    }
    if map.application_id != 20231
        || scene.schema_version != 1
        || map.source.retail_elf_sha256 != RETAIL_ELF_SHA256
        || scene.source_elf_sha256 != map.source.retail_elf_sha256
        || scene.map_name != map.map_name
        || scene.source_archive_sha256 != map.source.level_archive_sha256
        || scene.source_clv_sha256 != map.source.level_clv_sha256
        || scene.world_y_bounds != map.vertical_bounds
        || ![
            &map.source.level_archive_sha256,
            &map.source.level_clv_sha256,
            &map.source.terrain_resource_bank_sha256,
            &map.source.gameplay_objectives_sha256,
            &map.source.qualified_objective_poses_sha256,
            &scene.source_world_sha256,
            &scene.source_assembly_sha256,
            &scene.source_root_inventory_sha256,
            &scene.world_sha256,
        ]
        .into_iter()
        .all(|hash| hash_ok(hash))
    {
        return Err(invalid("active_ability_map_source_mismatch"));
    }
    let mut seen = BTreeSet::new();
    let meshes = scene
        .source_mesh_filter_counts
        .iter()
        .try_fold(0_usize, |count, row| {
            if row.root_filter_info >= 32
                || row.child_filter_info >= 32
                || row.mesh_count == 0
                || !seen.insert((row.root_filter_info, row.child_filter_info))
            {
                return None;
            }
            count.checked_add(row.mesh_count)
        });
    if meshes != Some(scene.source_mesh_count)
        || scene.root_count == 0
        || scene.root_count > scene.source_mesh_count
        || scene.output_mesh_count == 0
        || scene.output_mesh_count > scene.source_mesh_count
        || scene.reference_parent_root_count > scene.root_count
    {
        return Err(invalid("active_ability_static_ray_coverage_invalid"));
    }
    Ok(())
}

struct Loaded {
    setup: Setup,
    world_sha256: String,
    scene_mesh_count: usize,
    gameplay_objectives_sha256: String,
}

fn load_verified(path: &Path) -> io::Result<Loaded> {
    let manifest: Manifest = read(path)?;
    if manifest.format != "mag_active_abilities_v1" {
        return Err(invalid("invalid_active_ability_manifest"));
    }
    let rules: Rules = read(&manifest.rules)?;
    rules.validate().map_err(invalid)?;
    let effects: Effects = read(&manifest.effects)?;
    let map = decode_map_bindings(&read_bounded(&manifest.map_bindings)?).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("{}: {error}", manifest.map_bindings.display()),
        )
    })?;
    let scene: StaticRayReceipt = read(&manifest.static_ray_receipt)?;
    validate_sources(&rules, &effects, &map, &scene)?;
    let rows = effects
        .rows
        .into_iter()
        .map(|row| {
            let (ability, delay) = match row.rules {
                EffectRules::Cluster(rule) => (Ability::ClusterBomb, rule.delay_ms),
                EffectRules::Sensor(rule) => (Ability::Sensor, rule.delay_ms),
            };
            if rules.property(ability).map(|rule| rule.delay_ms) != Some(u64::from(delay)) {
                return Err(invalid("active_ability_effect_delay_mismatch"));
            }
            Ok((row.side, row.faction.wire_id(), ability, row.rules))
        })
        .collect::<io::Result<Vec<_>>>()?;
    let setup = Setup {
        rules,
        map_name: map.map_name,
        boundary: map.boundary,
        vertical_bounds: scene.world_y_bounds,
        aaa: map.aaa,
        objectives: map.objectives,
        damage_triggers: map.damage_triggers,
        effects: rows,
    };
    setup.validate().map_err(invalid)?;
    Ok(Loaded {
        setup,
        world_sha256: scene.world_sha256,
        scene_mesh_count: scene.output_mesh_count,
        gameplay_objectives_sha256: map.source.gameplay_objectives_sha256,
    })
}

/// Load and validate the explicit manifest and its source-bound inputs.
pub fn load(path: impl AsRef<Path>) -> io::Result<Setup> {
    Ok(load_verified(path.as_ref())?.setup)
}

/// Validate against the specific preloaded collision and objective catalogs. No
/// file reads are needed to install the setup during formation selection.
pub fn load_for_catalogs(
    path: &Path,
    spawns: &SpawnCatalog,
    collision: &CollisionWorld,
    objectives: &ObjectiveCatalog,
    objective_path: &Path,
) -> io::Result<Setup> {
    let loaded = load_verified(path)?;
    let setup = loaded.setup;
    if !setup.damage_triggers.is_empty() && spawns.match_profile().game_type != 2 {
        return Err(invalid(
            "structure_damage_requires_native_domination_objective_lifecycle",
        ));
    }
    if setup.map_name != spawns.map_name()
        || !matches!(spawns.match_profile().game_type, 1 | 2)
        || collision.metadata().map_name != setup.map_name
        || collision.stats().triangles == 0
        || collision.stats().instances != loaded.scene_mesh_count
        || collision.source_sha256() != loaded.world_sha256
    {
        return Err(invalid("active_ability_world_binding_mismatch"));
    }
    if format!("{:x}", Sha256::digest(read_bounded(objective_path)?))
        != loaded.gameplay_objectives_sha256
    {
        return Err(invalid("active_ability_objective_source_mismatch"));
    }
    let indices: BTreeSet<_> = objectives.bound_objects().map(|(_, index)| index).collect();
    let mut seen = BTreeSet::new();
    if setup.aaa.is_empty()
        || setup
            .aaa
            .iter()
            .any(|row| !indices.contains(&row.object_index))
        || setup
            .objectives
            .iter()
            .any(|row| !indices.contains(&row.object_index) || !seen.insert(row.object_index))
    {
        return Err(invalid("active_ability_objective_binding_mismatch"));
    }
    validate_damage_trigger_bindings(&setup.damage_triggers, &objectives.objects)?;
    Ok(setup)
}

fn validate_damage_trigger_bindings(rules: &[Rule], objectives: &[Objective]) -> io::Result<()> {
    for rule in rules {
        if !objectives.iter().any(|objective| {
            objective.object_index == rule.object_index
                && objective.native_class == Some(NativeClass::Structural)
                && objective
                    .demolition
                    .as_ref()
                    .is_some_and(|demolition| rule.trigger_index < demolition.trigger_count)
        }) {
            return Err(invalid("active_ability_damage_trigger_binding_mismatch"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map_bindings_fixture(version: u32) -> serde_json::Value {
        json!({
            "schema_version": version,
            "application_id": 20231,
            "map_name": "synthetic",
            "source": {
                "retail_elf_sha256": "synthetic",
                "level_archive_sha256": "synthetic",
                "level_clv_sha256": "synthetic",
                "terrain_resource_bank_sha256": "synthetic",
                "gameplay_objectives_sha256": "synthetic",
                "qualified_objective_poses_sha256": "synthetic"
            },
            "boundary": {"min_x": 0.0, "max_x": 1.0, "min_z": 0.0, "max_z": 1.0},
            "vertical_bounds": [0.0, 1.0],
            "aaa": [],
            "objectives": []
        })
    }

    #[test]
    fn legacy_map_bindings_require_reprojection() {
        for version in [1, 2] {
            let error =
                decode_map_bindings(&map_bindings_fixture(version).to_string().into_bytes())
                    .err()
                    .expect("missing damage triggers must require reprojection");
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(error.to_string().contains("require_reprojection"));
        }
        let mut map = map_bindings_fixture(1);
        map["damage_triggers"] = json!([]);
        let error = decode_map_bindings(&map.to_string().into_bytes())
            .err()
            .unwrap();
        assert!(error.to_string().contains("require_reprojection"));
    }

    #[test]
    fn schema_two_allows_an_explicit_empty_trigger_list() {
        let mut map = map_bindings_fixture(2);
        map["damage_triggers"] = json!([]);
        let decoded = decode_map_bindings(&map.to_string().into_bytes()).unwrap();
        assert!(decoded.damage_triggers.is_empty());
    }

    #[test]
    fn triggers_bind_only_to_structural_demolition_slots() {
        let mut map = map_bindings_fixture(2);
        map["damage_triggers"] = json!([{
            "object_index": 11, "trigger_index": 0, "trigger_type": 4,
            "start_active": true, "damage_threshold": 60.0, "position": [1.0, 2.0, 3.0],
            "box": {"centre": [1.0, 2.0, 3.0], "half_extents": [1.0, 1.0, 1.0],
                    "axes": [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]}
        }]);
        let map = decode_map_bindings(&map.to_string().into_bytes()).unwrap();
        let mut objective: Objective = serde_json::from_value(json!({
            "token": "0x00000001", "object_index": 11, "initial_state": 0, "native_class": "structural",
            "demolition": {"trigger_count": 1, "owner_team": 0, "arm_ms": 1, "detonate_ms": 1,
                "defuse_ms": 1, "can_defuse": true, "activation_score": 0, "deactivation_score": 0,
                "position": [1.0, 2.0, 3.0], "maximum_distance_m": 1.0, "distance_policy": "synthetic"}
        })).unwrap();
        let rules = &map.damage_triggers;
        assert!(validate_damage_trigger_bindings(rules, std::slice::from_ref(&objective)).is_ok());
        assert!(validate_damage_trigger_bindings(rules, &[]).is_err());
        objective.native_class = Some(NativeClass::SecureHold);
        assert!(validate_damage_trigger_bindings(rules, std::slice::from_ref(&objective)).is_err());
        objective.native_class = Some(NativeClass::Structural);
        objective.demolition.as_mut().unwrap().trigger_count = 0;
        assert!(validate_damage_trigger_bindings(rules, std::slice::from_ref(&objective)).is_err());
    }
}
