//! Load a source-bound Acquisition configuration for one selected match profile.
use crate::{
    acquisition_control::Catalog,
    acquisition_gameplay::{Fsp, Gameplay, Objective, OrderSource, Squad},
    acquisition_gameplay_engine::Setup,
    acquisition_ground_runtime::{Capabilities, Pose, QualifiedRespawn, Runtime},
    acquisition_objective_runtime::{Assignment, State},
    acquisition_round::Rules,
    acquisition_world::Settings,
    dynamic_collision::{self, QueryBox},
    ground_vehicles::arrival_static::Binding,
    objective_catalog::ObjectiveCatalog,
    objective_range::TriggerBox,
    replication::{CharacterPosition, ObjectiveControlDelta, ObjectiveState, ObjectiveStateActor},
    spawn_catalog::SpawnCatalog,
};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

const MAX_BYTES: usize = 65_536;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    format: String,
    schema_version: u32,
    source: Source,
    controller_path: PathBuf,
    rules: RuleData,
    filter_rows: [u32; 32],
    objectives: Vec<ObjectiveData>,
    squads: Vec<SquadData>,
    volumes: Vec<VolumeData>,
    arrival_triggers: Vec<ArrivalTrigger>,
    arrival_static: Option<Binding>,
    ground_capabilities: Vec<GroundCapability>,
    ground_damage: Option<crate::acquisition_ground_damage::Policy>,
    vehicle_reinforcement_intervals_ms: [u32; 2],
    structural: Option<crate::acquisition_structural::Files>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    map_name: String,
    spawn_catalog_sha256: String,
    controller_sha256: String,
    objective_binding_sha256: String,
    startup_proof_sha256: String,
    native_rules_sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleData {
    starting_points: [i16; 2],
    major_percent: i32,
    end_delay_ms: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ObjectiveClass {
    Capture,
    Arrival,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ObjectiveData {
    token: u32,
    object_index: u16,
    owner_team: u8,
    #[serde(rename = "class")]
    class: ObjectiveClass,
    initial_active: bool,
    activation_score_value: u16,
    fsp: Option<FspData>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FspData {
    vehicle_index: u16,
    spawn_point_index: u16,
    spawn_point_token: u32,
    position: CharacterPosition,
    rotation: [f32; 4],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SquadData {
    squad_index: u16,
    team: u8,
    gate_value: u16,
    normal: Option<Vec<OrderData>>,
    frago: Vec<OrderData>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OrderData {
    objective_token: Option<u32>,
    order: i32,
}
impl From<&OrderData> for OrderSource {
    fn from(input: &OrderData) -> Self {
        Self {
            objective_token: input.objective_token,
            order: input.order,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VolumeData {
    token: u32,
    pose: VolumePose,
    half_extents: [f64; 3],
    radius: f64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VolumePose {
    translation: [f64; 3],
    rotation: [[f64; 3]; 3],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArrivalTrigger {
    objective_token: u32,
    volume: TriggerBox,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GroundCapability {
    vehicle_index: u16,
    capabilities: Capabilities,
}

pub(crate) fn configure_ground(
    runtime: &mut Runtime,
    capabilities: &[(u16, Capabilities)],
) -> Result<(), &'static str> {
    for (index, capabilities) in capabilities {
        let key = runtime
            .get(*index)
            .ok_or("acquisition_ground_capability_vehicle_missing")?
            .key;
        runtime.configure(key, capabilities.clone())?;
    }
    Ok(())
}

impl Configuration {
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let mut text = String::new();
        let path = path.as_ref();
        File::open(path)?
            .take(MAX_BYTES as u64 + 1)
            .read_to_string(&mut text)?;
        let mut result = Self::from_json(&text)?;
        if let Some(structural) = &mut result.structural {
            structural.resolve(
                path.parent()
                    .ok_or_else(|| io::Error::other("acquisition_config_directory_missing"))?,
            );
        }
        Ok(result)
    }

    pub fn from_json(text: &str) -> io::Result<Self> {
        if text.len() > MAX_BYTES {
            return Err(io::Error::other("acquisition_configuration_too_large"));
        }
        let configuration: Self = serde_json::from_str(text)?;
        configuration.validate().map_err(io::Error::other)?;
        Ok(configuration)
    }

    fn validate(&self) -> Result<(), &'static str> {
        if self.ground_damage.is_none() || self.arrival_static.is_none() {
            return Err("acquisition_core_resources_required");
        }
        if self.format != "mag_acquisition_gameplay"
            || self.schema_version != 1
            || !self.controller_path.is_absolute()
            || self.source.map_name.is_empty()
            || self.source.map_name.len() > 128
            || [
                &self.source.spawn_catalog_sha256,
                &self.source.controller_sha256,
                &self.source.objective_binding_sha256,
                &self.source.startup_proof_sha256,
                &self.source.native_rules_sha256,
            ]
            .into_iter()
            .any(|hash| {
                hash.len() != 64
                    || !hash
                        .bytes()
                        .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
            })
            || self.rules.starting_points.iter().any(|points| *points <= 0)
            || !(0..=100).contains(&self.rules.major_percent)
            || !(1..=60_000).contains(&self.rules.end_delay_ms)
            || self.vehicle_reinforcement_intervals_ms.contains(&0)
            || self.arrival_static.as_ref().is_some_and(|binding| {
                !binding.path.is_absolute()
                    || binding.sha256.len() != 64
                    || !binding
                        .sha256
                        .bytes()
                        .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
            })
        {
            return Err("invalid_acquisition_configuration_identity");
        }
        if let Some(policy) = &self.ground_damage {
            policy.validate()?;
        }
        if !(8..=128).contains(&self.objectives.len())
            || self.squads.len() != 16
            || self.volumes.len() != 4
            || self.arrival_triggers.len() != 4
            || !(4..=32).contains(&self.ground_capabilities.len())
        {
            return Err("invalid_acquisition_configuration_inventory");
        }
        Ok(())
    }

    pub fn build(
        &self,
        spawns: &SpawnCatalog,
        objective_catalog: &ObjectiveCatalog,
        ground: &Runtime,
    ) -> io::Result<Setup> {
        let controller = Catalog::load(&self.controller_path, spawns)?;
        let mut setup = self
            .build_profile(controller, spawns, objective_catalog, ground)
            .map_err(io::Error::other)?;
        setup.structural = self
            .structural
            .as_ref()
            .map(|paths| paths.load(objective_catalog, spawns))
            .transpose()?;
        if let Some(structural) = &setup.structural {
            structural
                .validate_ground(ground)
                .map_err(io::Error::other)?;
        }
        Ok(setup)
    }

    fn build_profile(
        &self,
        controller: Catalog,
        spawns: &SpawnCatalog,
        objective_catalog: &ObjectiveCatalog,
        ground: &Runtime,
    ) -> Result<Setup, &'static str> {
        let objective_source = objective_catalog.source();
        objective_catalog.validate_match(&self.source.map_name, spawns.match_profile())?;
        if spawns.map_pool().name != self.source.map_name
            || self.rules.starting_points != spawns.match_profile().maximum_points
            || self.vehicle_reinforcement_intervals_ms != spawns.match_profile().spawn_wave_ms
            || spawns.source_sha256() != Some(&self.source.spawn_catalog_sha256)
            || controller.fingerprint() != self.source.controller_sha256
            || objective_source.capture_qualification != "native_source_gameplay"
            || objective_source.binding_capture_sha256 != self.source.objective_binding_sha256
            || objective_source.startup_proof_sha256 != self.source.startup_proof_sha256
            || objective_source.rules_catalog_sha256.as_ref()
                != Some(&self.source.native_rules_sha256)
        {
            return Err("acquisition_configuration_source_mismatch");
        }
        let expected_captures: BTreeSet<_> = controller
            .chains()
            .iter()
            .map(|c| c.capture_token)
            .collect();
        let expected_arrivals: BTreeSet<_> = controller
            .chains()
            .iter()
            .map(|c| c.arrival_token)
            .collect();
        let expected_volumes: BTreeSet<_> =
            controller.chains().iter().map(|c| c.volume_token).collect();
        let mut objectives = BTreeMap::new();
        let interaction_owners: BTreeMap<_, _> = objective_catalog
            .objects
            .iter()
            .filter_map(|object| {
                object
                    .interaction
                    .as_ref()
                    .map(|rule| (object.object_index, rule.owner_team))
            })
            .collect();
        if interaction_owners.len() != expected_captures.len() {
            return Err("acquisition_configuration_interaction_mismatch");
        }
        let mut initial_active = Vec::new();
        for row in &self.objectives {
            if row.token == 0
                || objective_catalog.binding(row.token) != Some((row.object_index, 0))
                || (row.class == ObjectiveClass::Arrival) != expected_arrivals.contains(&row.token)
                || row.initial_active != (row.class == ObjectiveClass::Capture)
                || row.fsp.is_some() != expected_captures.contains(&row.token)
                || (expected_captures.contains(&row.token)
                    && interaction_owners.get(&row.object_index) != Some(&row.owner_team))
                || (row.class == ObjectiveClass::Capture && row.activation_score_value != 0)
            {
                return Err("acquisition_configuration_objective_mismatch");
            }
            let fsp = row
                .fsp
                .as_ref()
                .map(|input| {
                    let point = spawns
                        .point(input.spawn_point_index)
                        .ok_or("acquisition_fsp_source_missing")?;
                    let vehicle = ground
                        .get(input.vehicle_index)
                        .ok_or("acquisition_fsp_vehicle_missing")?;
                    let pose = Pose {
                        position: input.position,
                        rotation: input.rotation,
                    };
                    if point.active
                        || point.token != input.spawn_point_token
                        || vehicle.state != 0
                        || vehicle.source_spawn_type != Some(9)
                        || vehicle.source_spawn_point_index != point.index
                        || vehicle.source_spawn_point_token != point.token
                        || vehicle.pose != pose
                    {
                        return Err("acquisition_fsp_resource_mismatch");
                    }
                    Ok(Fsp {
                        vehicle: vehicle.key,
                        source: QualifiedRespawn {
                            spawn_point_index: point.index,
                            spawn_point_token: point.token,
                            pose,
                            active: false,
                        },
                    })
                })
                .transpose()?;
            if row.initial_active {
                initial_active.push((row.object_index, row.token));
            }
            let objective = Objective {
                state: State {
                    current: 0,
                    target: 0,
                },
                owner_team: row.owner_team,
                activation_score_value: row.activation_score_value,
                current_attacker: u32::MAX,
                current_defender: u32::MAX,
                replication: ObjectiveControlDelta {
                    object_index: row.object_index,
                    state_actor: ObjectiveStateActor {
                        state: ObjectiveState::Inactive,
                        change_time_ms: 0,
                        actor_index: u16::MAX,
                    },
                    expiration_time_ms: 0,
                    deactivation_time_ms: 0,
                    health_percent: 100,
                },
                fsp,
            };
            if objectives.insert(row.token, objective).is_some() {
                return Err("acquisition_configuration_duplicate_objective");
            }
        }
        initial_active.sort_unstable();
        let initial_active = initial_active.into_iter().map(|(_, token)| token).collect();

        let mut squads = BTreeMap::new();
        for row in &self.squads {
            let authored = objective_catalog
                .authored_orders(row.squad_index)
                .ok_or("acquisition_configuration_squad_mismatch")?;
            let normal = row
                .normal
                .as_ref()
                .ok_or("acquisition_configuration_squad_mismatch")?;
            let projected = normal.iter().map(|order| {
                order
                    .objective_token
                    .and_then(|token| objectives.get(&token))
                    .map(|objective| (objective.replication.object_index, order.order as u32))
            });
            if row.team != (row.squad_index / 16) as u8
                || normal.len() > 128
                || row.frago.len() > 128
                || projected.ne(authored.map(Some))
                || squads
                    .insert(
                        row.squad_index,
                        Squad {
                            team: row.team,
                            gate_value: row.gate_value,
                            // Engine resolves the live selected spawn before refresh.
                            spawn_resolved: false,
                            normal: Some(normal.iter().map(OrderSource::from).collect()),
                            frago: row.frago.iter().map(OrderSource::from).collect(),
                            manual_tag: u32::MAX,
                            assignment: Assignment {
                                primary_index: u16::MAX,
                                tagged_object: u32::MAX,
                            },
                        },
                    )
                    .is_some()
            {
                return Err("acquisition_configuration_squad_mismatch");
            }
        }
        let state = Gameplay::new(&controller, objectives, squads)?;
        let mut volumes = BTreeMap::new();
        for row in &self.volumes {
            let pose = dynamic_collision::Pose {
                translation: row.pose.translation,
                rotation: row.pose.rotation,
            };
            pose.validate()?;
            if row
                .half_extents
                .iter()
                .any(|v| !v.is_finite() || *v <= 0. || *v > 10_000.)
                || row.radius != f64::from(f32::from_bits(0x3d4ccccd))
                || !expected_volumes.contains(&row.token)
                || volumes
                    .insert(
                        row.token,
                        QueryBox {
                            pose,
                            half_extents: row.half_extents,
                            radius: row.radius,
                        },
                    )
                    .is_some()
            {
                return Err("invalid_acquisition_configuration_volume");
            }
        }
        let mut arrival_triggers = BTreeMap::new();
        for row in &self.arrival_triggers {
            if !expected_arrivals.contains(&row.objective_token)
                || !crate::objective_range::ObjectiveRange::validate(
                    crate::objective_range::AUTHORED_TRIGGER_BOX,
                    1.,
                    1,
                    &[row.volume],
                )
                || arrival_triggers
                    .insert(row.objective_token, row.volume)
                    .is_some()
            {
                return Err("invalid_acquisition_configuration_arrival_trigger");
            }
        }
        for (a, row) in self.filter_rows.iter().enumerate() {
            if self
                .filter_rows
                .iter()
                .enumerate()
                .any(|(b, other)| (row & (1 << b) != 0) != (other & (1 << a) != 0))
            {
                return Err("invalid_acquisition_configuration_filter_rows");
            }
        }
        let arrival_static = self
            .arrival_static
            .as_ref()
            .map(|binding| binding.load(Path::new("/"), &self.source.map_name))
            .transpose()?;
        let expected_vehicles: BTreeSet<_> =
            ground.vehicles().map(|vehicle| vehicle.key.index).collect();
        let covered_vehicles: BTreeSet<_> = self
            .ground_capabilities
            .iter()
            .map(|row| row.vehicle_index)
            .collect();
        if covered_vehicles != expected_vehicles
            || covered_vehicles.len() != self.ground_capabilities.len()
        {
            return Err("acquisition_ground_capability_coverage_mismatch");
        }
        let ground_capabilities: Vec<_> = self
            .ground_capabilities
            .iter()
            .map(|row| (row.vehicle_index, row.capabilities.clone()))
            .collect();
        if self.ground_capabilities.iter().any(|row| {
            let vehicle = ground.get(row.vehicle_index).unwrap();
            row.capabilities.destruction.is_none()
                || row.capabilities.hulk_template.is_none()
                || row.capabilities.delivered_inert_seconds.is_some()
                    != (vehicle.source_spawn_type == Some(9))
        }) {
            return Err("acquisition_ground_capability_source_mismatch");
        }
        configure_ground(&mut ground.clone(), &ground_capabilities)?;
        Ok(Setup {
            controller,
            state,
            rules: Rules {
                starting_points: self.rules.starting_points,
                major_percent: self.rules.major_percent,
                end_delay_ms: self.rules.end_delay_ms,
            },
            filter_rows: self.filter_rows,
            physics: Settings::default(),
            volumes,
            initial_active,
            arrival_triggers,
            arrival_static,
            ground_capabilities,
            ground_damage: self.ground_damage.clone(),
            vehicle_reinforcement_intervals_ms: self.vehicle_reinforcement_intervals_ms,
            structural: None,
        })
    }
}
