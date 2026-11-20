//! Detached structural and squad effects with explicit provider-owned world operations.
use crate::{
    acquisition_objective_runtime::{Assignment, Objective, Order, Refresh, RefreshResult},
    domination_structural_effects::{
        BlastProperties, Catalog, CollisionResourceRef, PlaybackChange,
    },
    objective_catalog::{NativeClass, ObjectiveCatalog},
    objective_control::{NativeEntry, ObjectiveWorld},
    replication::{MatchPhase, ObjectiveSequenceDelta},
    spawn_catalog::SpawnCatalog,
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug)]
pub struct SquadFacts<'a> {
    pub assignment: Assignment,
    pub gate_value: u16,
    /// Original live point lookup. Authored eligibility alone is insufficient.
    pub spawn_resolved: bool,
    pub manual_tag: u32,
    pub frago: &'a [(Option<u16>, i32)],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AssignmentWrite {
    pub squad_index: u16,
    pub assignment: Assignment,
    pub result: RefreshResult,
}

/// Owned squad assignment authority for the replacement map. Each array slot
/// keeps its own manual FRAGO override and replicated selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnedAssignment {
    pub assignment: Assignment,
    pub manual_tag: u32,
}

impl OwnedAssignment {
    pub fn initial(catalog: &ObjectiveCatalog, squad_index: u16) -> Result<Self, &'static str> {
        Ok(Self {
            assignment: Assignment {
                primary_index: catalog.initial_assignment(squad_index)?,
                tagged_object: u32::MAX,
            },
            manual_tag: u32::MAX,
        })
    }

    pub(crate) fn with_manual_tag(
        mut self,
        squad: u16,
        target: u32,
        catalog: &ObjectiveCatalog,
        world: &ObjectiveWorld,
        spawns: &SpawnCatalog,
        phase: MatchPhase,
    ) -> Result<Self, &'static str> {
        self.manual_tag = target;
        let mut slots = [None; 32];
        *slots
            .get_mut(usize::from(squad))
            .ok_or("domination_assignment_slot_missing")? = Some(self);
        if let Some(update) = stage_owned_assignments(catalog, world, spawns, &slots, phase)?.pop()
        {
            self.assignment = update.assignment;
        }
        Ok(self)
    }
}

/// Refresh all admitted squad slots from the staged live family-35 catalog.
/// Retail Domination authors empty FRAGO vectors in all three source maps;
/// manual overrides are independent runtime state.
pub fn stage_owned_assignments(
    catalog: &ObjectiveCatalog,
    world: &ObjectiveWorld,
    spawns: &SpawnCatalog,
    squads: &[Option<OwnedAssignment>; 32],
    phase: MatchPhase,
) -> Result<Vec<AssignmentWrite>, &'static str> {
    if spawns.match_profile().game_type != 2 || !spawns.map_pool().runtime_indices_verified {
        return Err("domination_assignments_require_native_spawn_catalog");
    }
    let facts = std::array::from_fn(|index| {
        squads[index].map(|squad| SquadFacts {
            assignment: squad.assignment,
            gate_value: phase as u16,
            spawn_resolved: spawns
                .default_for_squad(index as u16)
                .and_then(|point| spawns.point(point))
                .is_some(),
            manual_tag: squad.manual_tag,
            frago: &[],
        })
    });
    stage_squad_assignments(catalog, world, &facts, |index, team| {
        let object = catalog
            .objects
            .iter()
            .find(|object| object.object_index == index)
            .ok_or("domination_assignment_objective_missing")?;
        let owner = object
            .interaction
            .as_ref()
            .map(|rule| rule.owner_team)
            .or_else(|| object.demolition.as_ref().map(|rule| rule.owner_team))
            .ok_or("domination_assignment_owner_missing")?;
        Ok(team != owner)
    })
}

/// Apply the native assignment predicate to source orders without changing
/// authority. Vacant slots are explicit and all occupied slots read the same
/// current-state observation.
pub fn stage_squad_assignments(
    catalog: &ObjectiveCatalog,
    world: &ObjectiveWorld,
    facts: &[Option<SquadFacts<'_>>; 32],
    mut is_attacker: impl FnMut(u16, u8) -> Result<bool, &'static str>,
) -> Result<Vec<AssignmentWrite>, &'static str> {
    if !catalog.is_domination_gameplay() {
        return Err("domination_assignments_require_gameplay_catalog");
    }
    let mut updates = Vec::new();
    for (slot, facts) in facts.iter().enumerate() {
        let Some(facts) = facts else { continue };
        if facts.gate_value == 3 || !facts.spawn_resolved {
            continue;
        }
        let squad_index = slot as u16;
        let normal = catalog
            .authored_orders(squad_index)
            .ok_or("missing_domination_assignment_source_orders")?;
        let mut resolve = |index: Option<u16>, order| {
            let objective = index
                .map(|index| {
                    let state = world
                        .state(index)
                        .ok_or("missing_domination_assignment_objective")?;
                    Objective::new(
                        0x0c80_0000 | u32::from(index),
                        state as i32,
                        is_attacker(index, (slot / 16) as u8)?,
                    )
                })
                .transpose()?;
            Ok(Order { objective, order })
        };
        let normal = normal
            .map(|(index, order)| resolve(Some(index), order as i32))
            .collect::<Result<Vec<_>, &'static str>>()?;
        let frago = facts
            .frago
            .iter()
            .map(|&(index, order)| resolve(index, order))
            .collect::<Result<Vec<_>, &'static str>>()?;
        let mut assignment = facts.assignment;
        let result = assignment.refresh(Refresh {
            gate_value: facts.gate_value,
            spawn_resolved: facts.spawn_resolved,
            normal: Some(&normal),
            frago: &frago,
            manual_tag: facts.manual_tag,
        });
        updates.push(AssignmentWrite {
            squad_index,
            assignment,
            result,
        });
    }
    Ok(updates)
}

#[derive(Clone, Copy, Debug)]
pub struct ExplosionRequest {
    pub objective_token: u32,
    pub entry: NativeEntry,
    pub properties: BlastProperties,
}

/// Current selected playback per placed Structural objective. This copy is
/// installed with the accepted collision/animation world only after recording.
#[derive(Clone)]
pub struct Structural {
    catalog: Catalog,
    tokens: BTreeMap<u16, (u32, NativeClass)>,
    selected: BTreeMap<u16, ObjectiveSequenceDelta>,
}

impl Structural {
    pub(crate) fn reset(&mut self) {
        self.selected.clear();
    }

    pub(crate) fn blast_properties(&self, index: u16) -> Result<BlastProperties, &'static str> {
        let (token, class) = self
            .tokens
            .get(&index)
            .ok_or("domination_structural_effect_objective_mismatch")?;
        if *class != NativeClass::Structural {
            return Err("domination_structural_effect_objective_mismatch");
        }
        self.catalog.blast_properties(*token)
    }

    pub(crate) fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    pub(crate) fn material_penetration_costs(&self) -> &BTreeMap<u8, f32> {
        self.catalog.material_penetration_costs()
    }

    pub(crate) fn current_sequences(&self) -> Vec<ObjectiveSequenceDelta> {
        self.selected.values().copied().collect()
    }

    pub fn from_json(
        raw: &str,
        objectives: &ObjectiveCatalog,
        map: &str,
        level_archive_sha256: &str,
        level_clv_sha256: &str,
    ) -> Result<Self, &'static str> {
        let catalog = Catalog::from_json(raw)?;
        if !objectives.is_domination_gameplay()
            || catalog.metadata_sha256 != objectives.source().authored_catalog_sha256
            || catalog.map != map
            || catalog.level_archive_sha256 != level_archive_sha256
            || catalog.level_clv_sha256 != level_clv_sha256
        {
            return Err("domination_structural_effect_source_mismatch");
        }
        let mut tokens = BTreeMap::new();
        for (token, index) in objectives.bound_objects() {
            let class = objectives
                .objects
                .iter()
                .find(|object| object.object_index == index)
                .and_then(|object| object.native_class)
                .ok_or("domination_effect_objective_class_missing")?;
            catalog.objective_pose(token)?;
            if class == NativeClass::Structural {
                catalog.blast_properties(token)?;
            }
            tokens.insert(index, (token, class));
        }
        Ok(Self {
            catalog,
            tokens,
            selected: BTreeMap::new(),
        })
    }

    pub(crate) fn from_acquisition_catalog(
        catalog: Catalog,
        objectives: &ObjectiveCatalog,
    ) -> Result<Self, &'static str> {
        let mut tokens = BTreeMap::new();
        for token in catalog.placed_objectives() {
            let (index, _) = objectives
                .binding(token)
                .ok_or("acquisition_structural_effect_objective_mismatch")?;
            catalog.blast_properties(token)?;
            tokens.insert(index, (token, NativeClass::Structural));
        }
        Ok(Self {
            catalog,
            tokens,
            selected: BTreeMap::new(),
        })
    }

    fn validate_entry(
        &self,
        objective_token: u32,
        entry: NativeEntry,
    ) -> Result<u16, &'static str> {
        let index = entry.delta.object_index;
        if self.tokens.get(&index) != Some(&(objective_token, entry.class)) {
            return Err("domination_structural_effect_objective_mismatch");
        }
        Ok(index)
    }

    pub fn explosion(
        &self,
        objective_token: u32,
        entry: NativeEntry,
    ) -> Result<ExplosionRequest, &'static str> {
        self.validate_entry(objective_token, entry)?;
        if !entry.requires_authored_explosion() {
            return Err("domination_structural_explosion_not_requested");
        }
        Ok(ExplosionRequest {
            objective_token,
            entry,
            properties: self.catalog.blast_properties(objective_token)?,
        })
    }

    /// Prepare all stop/start geometry effects in source order before advancing
    /// this candidate's selected sequence. An unresolved collision asset still
    /// requires qualification and refuses the entire candidate.
    pub fn prepare_sequence(
        &mut self,
        objective_token: u32,
        entry: NativeEntry,
        game_time_ms: u32,
        prepare: impl FnOnce(&Catalog, PlaybackChange<'_>) -> Result<(), &'static str>,
    ) -> Result<Option<ObjectiveSequenceDelta>, &'static str> {
        let index = self.validate_entry(objective_token, entry)?;
        let prior = self
            .selected
            .get(&index)
            .map_or(-1, |delta| delta.sequence_index as i8);
        let plan = self
            .catalog
            .sequence(objective_token, entry.delta.state_actor.state, prior)?;
        let Some(ordinal) = plan.replicated_index else {
            return Ok(None);
        };
        if let Some(playback) = plan.change {
            if playback.starts.iter().any(|creation| {
                self.catalog.collision_reference(creation) == CollisionResourceRef::Unresolved
            }) {
                return Err("domination_structural_unresolved_collision_resource");
            }
            prepare(&self.catalog, playback)?;
        }
        let delta = ObjectiveSequenceDelta {
            object_index: index,
            sequence_index: i32::from(ordinal),
            start_time_ms: game_time_ms,
        };
        self.selected.insert(index, delta);
        Ok(Some(delta))
    }
}
