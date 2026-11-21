//! Native-order scheduling for an owned Domination callback map.
use crate::domination_control::{Effect, Invocation, Map, State, Transition};
use std::{collections::BTreeMap, num::NonZeroUsize, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Context {
    /// The committed objective entry and its original calling objective token.
    pub transition: Transition,
    /// Original Lua Caller object identity, normally the source objective.
    /// The called binding reads that object's current actor at invocation.
    pub caller_object_id: u32,
    pub invocation: Invocation,
}

#[derive(Debug, PartialEq, Eq)]
pub struct InvocationEffects {
    pub context: Context,
    pub effects: Vec<Effect>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pending {
    context: Context,
    due_ms: u32,
}

/// An owned queue for one immutable controller map. Its capacity is an engine
/// resource bound, independent of the original vector's allocation capacity.
#[derive(Clone, Debug)]
pub struct Queue {
    map: Arc<Map>,
    pending: Vec<Pending>,
    capacity: NonZeroUsize,
}

impl Queue {
    pub fn new(map: Arc<Map>, capacity: NonZeroUsize) -> Self {
        Self {
            map,
            pending: Vec::new(),
            capacity,
        }
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn clear(&mut self) {
        self.pending.clear();
    }

    /// Emit the immediate branch for an already committed entry, and retain
    /// positive-delay entries in their authored vector order. The consumer
    /// prepares effects in its own candidate world before the queue changes.
    pub fn execute_event<T>(
        &mut self,
        transition: Transition,
        caller_object_id: u32,
        game_time_ms: u32,
        current: &BTreeMap<u32, State>,
        prepare_immediate: impl FnOnce(InvocationEffects) -> Result<T, &'static str>,
    ) -> Result<Option<T>, &'static str> {
        let mut delayed = Vec::new();
        let mut immediate = None;
        for invocation in self
            .map
            .invocations(transition.objective_token, transition.event)
        {
            let context = Context {
                transition,
                caller_object_id,
                invocation,
            };
            if invocation.delay_ms == 0 {
                immediate = Some(context);
            } else {
                if delayed.len() == self.capacity.get() - self.pending.len() {
                    return Err("domination_callback_queue_full");
                }
                delayed.push(Pending {
                    context,
                    due_ms: game_time_ms.wrapping_add(invocation.delay_ms),
                });
            }
        }
        let prepared = immediate
            .map(|context| self.effects(context, current).and_then(prepare_immediate))
            .transpose()?;
        self.pending.extend(delayed);
        Ok(prepared)
    }

    /// Use the caller's sample of the network game clock for the entire update.
    /// The original scan skips future entries and erases due entries without
    /// reordering the rest of the vector. The engine can pass this same sample
    /// again with a new current-state snapshot after each accepted invocation.
    pub fn consume_next_due<T>(
        &mut self,
        game_time_ms: u32,
        current: &BTreeMap<u32, State>,
        prepare: impl FnOnce(InvocationEffects) -> Result<T, &'static str>,
    ) -> Result<Option<T>, &'static str> {
        let Some(index) = self
            .pending
            .iter()
            .position(|entry| entry.due_ms <= game_time_ms)
        else {
            return Ok(None);
        };
        let invocation = self.effects(self.pending[index].context, current)?;
        let prepared = prepare(invocation)?;
        self.pending.remove(index);
        Ok(Some(prepared))
    }

    fn effects(
        &self,
        context: Context,
        current: &BTreeMap<u32, State>,
    ) -> Result<InvocationEffects, &'static str> {
        let effects = self
            .map
            .effects(context.invocation, |token| current.get(&token).copied())?;
        Ok(InvocationEffects { context, effects })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domination_control::{Caller, Catalog, Event};
    use serde_json::json;

    const TRANSITION: Transition = Transition {
        objective_token: 1,
        event: Event::Completed,
    };
    const ACTOR: u32 = 0x0380_0010;

    fn queue(capacity: usize) -> Queue {
        let hash = "1".repeat(64);
        let entries: Vec<_> = [0, 20, 5, 20]
            .into_iter()
            .enumerate()
            .map(|(index, delay)| {
                json!({
                    "delay_ms": delay, "rules":[{
                        "conditions":[{"objective_token":2,"state":"completed","equal":true}],
                        "commands":[{"kind":"set_state","objective_token":3+index,
                                     "state":"locked","caller":"original"}],
                    }],
                })
            })
            .collect();
        let raw = json!({
            "format":"mag_domination_control","schema_version":1,
            "game_archive_sha256":hash,"metadata_sha256":hash,"script_sha256":[hash],
            "maps":[{
                "name":"synthetic","level_archive_sha256":hash,"level_clv_sha256":hash,
                "bindings":[{"binding_token":77,"entries":entries}],
                "hooks":[{"objective_token":1,"event":"completed","binding_token":77,"resolved":true}],
            }],
        });
        let catalog = Catalog::from_json(raw.to_string().as_bytes()).unwrap();
        Queue::new(
            Arc::new(catalog.map("synthetic").unwrap().clone()),
            NonZeroUsize::new(capacity).unwrap(),
        )
    }

    fn state(value: State) -> BTreeMap<u32, State> {
        BTreeMap::from([(2, value)])
    }

    fn target(token: u32) -> Effect {
        Effect::SetState {
            objective_token: token,
            state: State::Locked,
            caller: Caller::Original,
        }
    }

    #[test]
    fn immediate_and_delayed_branches_read_distinct_current_state_snapshots() {
        let mut queue = queue(8);
        let start = queue
            .execute_event(TRANSITION, ACTOR, 100, &state(State::Other), Ok)
            .unwrap()
            .unwrap();
        assert_eq!(
            start.context,
            Context {
                transition: TRANSITION,
                caller_object_id: ACTOR,
                invocation: Invocation {
                    binding_token: 77,
                    entry_index: 0,
                    delay_ms: 0
                },
            }
        );
        assert!(start.effects.is_empty());
        assert_eq!(queue.pending_count(), 3);
        assert_eq!(
            queue.consume_next_due(104, &BTreeMap::new(), Ok).unwrap(),
            None
        );

        // The initial vector entry has a later deadline. A scan can skip it.
        let delayed = queue
            .consume_next_due(105, &state(State::Completed), Ok)
            .unwrap()
            .unwrap();
        assert_eq!(delayed.context.invocation.entry_index, 2);
        assert_eq!(delayed.context.transition, TRANSITION);
        assert_eq!(delayed.context.caller_object_id, ACTOR);
        assert_eq!(delayed.effects, [target(5)]);
        assert_eq!(queue.pending_count(), 2);

        for index in [1, 3] {
            let output = queue
                .consume_next_due(120, &state(State::Completed), Ok)
                .unwrap()
                .unwrap();
            assert_eq!(output.context.invocation.entry_index, index);
        }
        assert_eq!(
            queue.consume_next_due(120, &BTreeMap::new(), Ok).unwrap(),
            None
        );
    }

    #[test]
    fn overdue_entries_follow_vector_order_while_due_equal_entries_stay_stable() {
        let mut queue = queue(8);
        queue
            .execute_event(TRANSITION, ACTOR, 100, &state(State::Completed), Ok)
            .unwrap();
        // No synthetic calls at the missed 105/120 times. Every eligible entry
        // is taken once in pending vector order using the same native clock sample.
        for index in [1, 2, 3] {
            let next = queue
                .consume_next_due(150, &state(State::Completed), Ok)
                .unwrap()
                .unwrap();
            assert_eq!(next.context.invocation.entry_index, index);
        }
        assert_eq!(
            queue
                .consume_next_due(150, &state(State::Completed), Ok)
                .unwrap(),
            None
        );
    }

    #[test]
    fn queue_errors_keep_pending_state_for_retry_or_reset() {
        let mut queue = queue(3);
        let current = state(State::Completed);
        assert_eq!(
            queue.execute_event(TRANSITION, ACTOR, 100, &current, |batch| {
                assert_eq!(batch.effects, [target(3)]);
                Err::<(), _>("immediate_stage_failed")
            }),
            Err("immediate_stage_failed")
        );
        assert_eq!(queue.pending_count(), 0);
        queue
            .execute_event(TRANSITION, ACTOR, 100, &current, |_| Ok(()))
            .unwrap();
        assert_eq!(
            queue.execute_event(TRANSITION, 0x0380_0005, 101, &current, |_| {
                panic!("overfull event must not reach consumer")
            }),
            Err::<Option<()>, _>("domination_callback_queue_full")
        );

        assert_eq!(
            queue.consume_next_due(120, &BTreeMap::new(), Ok),
            Err("missing_domination_objective_state")
        );
        assert_eq!(
            queue.consume_next_due(120, &current, |batch| {
                assert_eq!(batch.effects, [target(4)]);
                Err::<(), _>("delayed_stage_failed")
            }),
            Err("delayed_stage_failed")
        );
        assert_eq!(queue.pending_count(), 3);
        let retried = queue.consume_next_due(120, &current, Ok).unwrap().unwrap();
        assert_eq!(retried.context.caller_object_id, ACTOR);
        assert_eq!(retried.context.invocation.entry_index, 1);
        queue.clear();
        assert_eq!(queue.consume_next_due(120, &current, Ok).unwrap(), None);
    }

    #[test]
    fn clock_uses_native_u32_wrap_and_unsigned_due_comparison() {
        let mut queue = queue(3);
        queue
            .execute_event(TRANSITION, u32::MAX, u32::MAX - 9, &state(State::Other), Ok)
            .unwrap();
        let output = queue
            .consume_next_due(10, &state(State::Completed), Ok)
            .unwrap()
            .unwrap();
        assert_eq!(output.context.invocation.entry_index, 1);
        assert_eq!(output.context.caller_object_id, u32::MAX);
        assert_eq!(queue.pending_count(), 2);
    }
}
