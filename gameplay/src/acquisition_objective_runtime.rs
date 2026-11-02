//! Native-bound target requests and squad assignment state.
//!
//! These operate on an owned candidate, not Engine authority. Current-state
//! entry, OnChangeTargetStateId hooks, live reference resolution, setter side
//! effects and publication remain the callback backend's responsibility.
use crate::acquisition_control::ObjectiveOperation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    pub current: i32,
    pub target: i32,
}
impl State {
    /// Return the pending target request only when its current-state guard passes
    /// and its target differs. Never commit current state or bypass native hooks.
    pub fn request(self, operation: ObjectiveOperation) -> Option<i32> {
        let requested = match operation {
            ObjectiveOperation::Enable if self.current == 0 => 1,
            ObjectiveOperation::Disable if self.current != 0 => 0,
            ObjectiveOperation::Lock if self.current != 9 => 9,
            _ => return None,
        };
        (self.target != requested).then_some(requested)
    }
}

/// One resolved live objective. `attacker` is the result of its qualified native
/// attacker predicate for this squad's team, not a client-provided role. The
/// inherited SecureAndHold predicate compares team16 != ownerTeam16; other
/// objective subclasses may override it and require their own qualification.
#[derive(Clone, Copy, Debug)]
pub struct Objective {
    object_id: u32,
    current: i32,
    attacker: bool,
}
impl Objective {
    /// Admitted replacement domain only. Native candidate setters do not perform
    /// this family/index validation. This constructor does not certify liveness.
    pub fn new(object_id: u32, current: i32, attacker: bool) -> Result<Self, &'static str> {
        if object_id & 0xffff0000 != 0x0c800000 || object_id as u16 == u16::MAX {
            return Err("unsupported_assignment_objective_identity");
        }
        Ok(Self {
            object_id,
            current,
            attacker,
        })
    }
}

/// Source order records are neither sorted nor deduplicated. Null pointers and
/// unrecognized order enums are retained and evaluate false.
#[derive(Clone, Copy, Debug)]
pub struct Order {
    pub objective: Option<Objective>,
    /// Native eObjectiveOrder: 0 Take, 1 Hold. Other values never match.
    pub order: i32,
}
impl Order {
    pub fn is_valid(self) -> bool {
        let Some(objective) = self.objective else {
            return false;
        };
        let take = matches!(objective.current, 1 | 2 | 3 | 4 | 6);
        let hold = matches!(objective.current, 5 | 7 | 10 | 11 | 12);
        match (objective.attacker, self.order) {
            (true, 0) | (false, 1) => take,
            (true, 1) | (false, 0) => hold,
            _ => false,
        }
    }
}
pub fn first_valid(orders: &[Order]) -> Option<u32> {
    orders
        .iter()
        .find(|o| o.is_valid())
        .and_then(|o| o.objective.map(|v| v.object_id))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Assignment {
    pub primary_index: u16,
    pub tagged_object: u32,
}
#[derive(Clone, Copy)]
pub struct Refresh<'a> {
    /// Raw observed native gameplay-property gate. Value3 skips this function;
    /// a semantic mode name is intentionally not inferred from the field alone.
    pub gate_value: u16,
    /// The current replicated spawn index is non-FFFF and resolves a live valid
    /// spawnpoint. A configured static spawn catalog alone cannot prove this.
    pub spawn_resolved: bool,
    /// None means missing source objective-list pointer, an early return.
    pub normal: Option<&'a [Order]>,
    pub frago: &'a [Order],
    /// FFFFFFFF means no manual tag; any other full ID overrides FRAGO selection.
    pub manual_tag: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RefreshResult {
    pub skipped: bool,
    pub primary_changed: bool,
    /// Native calls SetTaggedObjectID even when its supplied value is unchanged.
    pub tag_written: bool,
}
impl Assignment {
    /// Refresh an isolated candidate's fields. Caller owns the complete global
    /// sweep, live source resolution, setter consequences and replication. This
    /// does not turn initial-load identities into post-start qualification.
    pub fn refresh(&mut self, inputs: Refresh<'_>) -> RefreshResult {
        let Some(normal) = inputs
            .normal
            .filter(|_| inputs.gate_value != 3 && inputs.spawn_resolved)
        else {
            return RefreshResult {
                skipped: true,
                primary_changed: false,
                tag_written: false,
            };
        };
        let primary = first_valid(normal).map_or(u16::MAX, |id| id as u16);
        let primary_changed = self.primary_index != primary;
        self.primary_index = primary;
        self.tagged_object = if inputs.manual_tag != u32::MAX {
            inputs.manual_tag
        } else {
            first_valid(inputs.frago).unwrap_or(u32::MAX)
        };
        RefreshResult {
            skipped: false,
            primary_changed,
            tag_written: true,
        }
    }
}
