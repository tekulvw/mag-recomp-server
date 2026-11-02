//! Isolated callback transactions; effect implementations remain explicit.
use crate::acquisition_control::{Catalog, Command, ObservedState, UnboundCall};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldKey {
    pub world_id: u32,
    pub incarnation: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnteredState {
    Completed,
    Deactivated,
}
impl EnteredState {
    pub fn observed(self) -> ObservedState {
        match self {
            Self::Completed => ObservedState::Completed,
            Self::Deactivated => ObservedState::Deactivated,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub objective_token: u32,
    pub state: EnteredState,
    pub actor_index: u16,
    pub timestamp_ms: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub world: WorldKey,
    pub event_id: u64,
    pub entry: Entry,
}
#[derive(Clone, Debug, Serialize)]
pub struct Audit {
    pub request: Request,
    pub backend: Value,
    pub controller_sha256: String,
    pub revision_before: u64,
    pub revision_after: u64,
    pub commands: Vec<Command>,
    pub trailing_calls: Vec<UnboundCall>,
}
pub struct Outcome<O> {
    pub output: O,
    pub replayed: bool,
    pub audit: Audit,
}
/// A stage must own isolated mutable candidate state. Preparing it must not
/// publish, write authoritative state, or retain shared mutable authority.
/// Installation must be infallible: validate all fallible work before recording.
/// All authority must be serialized through this adapter/its commit guard; an
/// independent writer may not change it while check/record/install is in flight.
pub trait EffectBackend {
    type Output: Clone;
    type Stage: CallbackStage<Output = Self::Output>;
    fn begin(&self) -> Result<Self::Stage, &'static str>;
    /// Recheck source revisions, entity generations and staged references before
    /// recording. This must not mutate authority or publish output.
    fn check(&self, stage: &Self::Stage) -> Result<(), &'static str>;
    fn install(&mut self, stage: Self::Stage);
}
/// Complete effect handlers are mandatory. Unsupported effects return an error;
/// an implementation must never return success after skipping part of a command.
pub trait CallbackStage {
    /// Publication data only. All authoritative damage/spawn/state effects must
    /// already belong to the staged candidate, never deferred into this output.
    /// Returning cached output must not reapply authoritative effects.
    type Output: Clone;
    /// Commit entry's current state and perform its complete pre-script effects,
    /// including qualified FSP/vehicle operations and native entry bookkeeping.
    fn enter(&mut self, entry: &Entry) -> Result<(), &'static str>;
    fn observations(&self) -> Result<BTreeMap<u32, ObservedState>, &'static str>;
    /// Preserve source order and collision-pair multiplicity for volume damage.
    /// Objective setters change target requests, not current-state predicates.
    fn command(&mut self, command: &Command) -> Result<(), &'static str>;
    fn trailing(&mut self, call: &UnboundCall) -> Result<(), &'static str>;
    fn assignments(&mut self, entry: &Entry) -> Result<(), &'static str>;
    /// Check identities, complete consequences, history capacity and publication
    /// limits. After success, installing the candidate may not fail.
    fn validate(&self) -> Result<(), &'static str>;
    fn audit(&self) -> Value;
    /// Exact bounded publication data to record and release after installation.
    fn output(&self) -> Self::Output;
}
#[derive(Clone)]
struct Committed<O> {
    request: Request,
    output: O,
    audit: Audit,
}
enum Candidate<S: CallbackStage> {
    Ready {
        stage: S,
        output: S::Output,
        audit: Audit,
        revision: u64,
    },
    Replay,
}
/// Only its originating adapter may commit a preparation. Fields are private so
/// callers cannot replace the world, revision, audit or staged effect candidate.
pub struct Prepared<S: CallbackStage> {
    origin: std::sync::Arc<()>,
    request: Request,
    candidate: Candidate<S>,
}
pub struct AtomicCallbacks<B: EffectBackend> {
    catalog: Catalog,
    backend: B,
    world: WorldKey,
    origin: std::sync::Arc<()>,
    revision: u64,
    highest_id: u64,
    replay_capacity: usize,
    committed: BTreeMap<u64, Committed<B::Output>>,
}
impl<B: EffectBackend> AtomicCallbacks<B> {
    /// Callback IDs are local monotonic event identities, not transport sequence
    /// numbers. Use a new world incarnation/adapter for a reset or new match.
    pub fn new(
        catalog: Catalog,
        backend: B,
        world: WorldKey,
        replay_capacity: usize,
    ) -> Result<Self, &'static str> {
        if world.world_id == 0 || world.incarnation == 0 || !(1..=4096).contains(&replay_capacity) {
            return Err("invalid_acquisition_callback_context");
        }
        Ok(Self {
            catalog,
            backend,
            world,
            origin: std::sync::Arc::new(()),
            revision: 0,
            highest_id: 0,
            replay_capacity,
            committed: BTreeMap::new(),
        })
    }
    pub fn backend(&self) -> &B {
        &self.backend
    }
    fn replay(&self, request: Request) -> Result<Option<&Committed<B::Output>>, &'static str> {
        if request.world != self.world || request.event_id == 0 {
            return Err("invalid_acquisition_callback_context");
        }
        if let Some(old) = self.committed.get(&request.event_id) {
            if old.request != request {
                return Err("acquisition_callback_id_reused");
            }
            return Ok(Some(old));
        }
        if request.event_id <= self.highest_id {
            return Err("acquisition_callback_before_replay_window");
        }
        Ok(None)
    }
    pub fn prepare(&self, request: Request) -> Result<Prepared<B::Stage>, &'static str> {
        let origin = std::sync::Arc::clone(&self.origin);
        if self.replay(request)?.is_some() {
            return Ok(Prepared {
                origin,
                request,
                candidate: Candidate::Replay,
            });
        }
        if !self
            .catalog
            .objective_tokens()
            .any(|token| token == request.entry.objective_token)
        {
            return Err("unknown_acquisition_callback_objective");
        }
        let next_revision = self
            .revision
            .checked_add(1)
            .ok_or("acquisition_callback_revision_exhausted")?;
        let mut stage = self.backend.begin()?;
        stage.enter(&request.entry)?;
        let states = stage.observations()?;
        if states.get(&request.entry.objective_token) != Some(&request.entry.state.observed()) {
            return Err("acquisition_callback_entry_state_not_committed");
        }
        let plan = self.catalog.plan(&states)?;
        for command in &plan.commands {
            stage.command(command)?;
            if stage.observations()? != states {
                return Err("acquisition_callback_current_state_changed");
            }
        }
        for call in &plan.unbound_calls {
            stage.trailing(call)?;
            if stage.observations()? != states {
                return Err("acquisition_callback_current_state_changed");
            }
        }
        stage.assignments(&request.entry)?;
        stage.validate()?;
        let audit = Audit {
            request,
            backend: stage.audit(),
            controller_sha256: self.catalog.fingerprint().into(),
            revision_before: self.revision,
            revision_after: next_revision,
            commands: plan.commands,
            trailing_calls: plan.unbound_calls,
        };
        let output = stage.output();
        Ok(Prepared {
            origin,
            request,
            candidate: Candidate::Ready {
                stage,
                output,
                audit,
                revision: self.revision,
            },
        })
    }
    /// Record exact staged audit/output before installing or releasing anything.
    /// On error the candidate is discarded. A replay returns cached output and
    /// audit without calling the recorder or any effect handler again.
    pub fn commit<F>(
        &mut self,
        prepared: Prepared<B::Stage>,
        mut record: F,
    ) -> Result<Outcome<B::Output>, &'static str>
    where
        F: FnMut(&Audit, &B::Output) -> Result<(), &'static str>,
    {
        if !std::sync::Arc::ptr_eq(&self.origin, &prepared.origin) {
            return Err("foreign_acquisition_callback_preparation");
        }
        if let Some(cached) = self.replay(prepared.request)? {
            return Ok(Outcome {
                output: cached.output.clone(),
                audit: cached.audit.clone(),
                replayed: true,
            });
        }
        let Candidate::Ready {
            stage,
            output,
            audit,
            revision,
        } = prepared.candidate
        else {
            return Err("expired_acquisition_callback_replay");
        };
        if revision != self.revision {
            return Err("stale_acquisition_callback_preparation");
        }
        self.backend.check(&stage)?;
        let cached = Committed {
            request: prepared.request,
            output: output.clone(),
            audit: audit.clone(),
        };
        record(&audit, &output)?;
        self.backend.install(stage);
        self.revision = audit.revision_after;
        self.highest_id = prepared.request.event_id;
        self.committed.insert(self.highest_id, cached);
        while self.committed.len() > self.replay_capacity {
            self.committed.pop_first();
        }
        Ok(Outcome {
            output,
            audit,
            replayed: false,
        })
    }
}
