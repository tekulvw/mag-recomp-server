//! Secure-and-hold interaction requests shared by qualified objective modes.
//! The owner of the world transaction resolves contacts and applies entry effects.

use crate::replication::ObjectiveState::{self, *};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rules {
    pub owner_team: u8,
    pub trigger_count: u8,
    pub can_deactivate: bool,
    /// False when a Completed point is permanent: the owner may still reverse
    /// an attacker's hold before completion, but never retake the result.
    pub can_retake: bool,
    pub activate_ms: u32,
    pub complete_ms: u32,
    pub deactivate_ms: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Actor {
    pub player: u16,
    pub generation: u64,
    pub team: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Activate,
    Deactivate,
    Release,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    revision: u64,
    source: ObjectiveState,
    target: ObjectiveState,
    actor: Actor,
    trigger: u8,
    release: bool,
}

impl Request {
    pub fn target(self) -> ObjectiveState {
        self.target
    }

    pub fn actor(self) -> Actor {
        self.actor
    }

    pub fn trigger_index(self) -> u8 {
        self.trigger
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub previous: ObjectiveState,
    pub current: ObjectiveState,
    pub actor: Actor,
    pub expiration_ms: Option<u32>,
    pub deactivation_ms: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Interaction {
    actor: Actor,
    trigger: u8,
}

#[derive(Clone, Debug)]
pub struct SecureHold {
    rules: Rules,
    current: ObjectiveState,
    pending: Option<Request>,
    revision: u64,
    holding: Option<Interaction>,
    completed_by: Option<Interaction>,
    expiration_ms: Option<u32>,
    deactivation_ms: Option<u32>,
    selected_trigger: Option<u8>,
}

impl SecureHold {
    pub fn new(initial: ObjectiveState, rules: Rules) -> Result<Self, &'static str> {
        if rules.owner_team > 1 || !matches!(initial, Inactive | Deactivated | Completed | Locked) {
            return Err("invalid_secure_hold_initial_state_or_owner");
        }
        Ok(Self {
            rules,
            current: initial,
            pending: None,
            revision: 0,
            holding: None,
            completed_by: None,
            expiration_ms: None,
            deactivation_ms: None,
            selected_trigger: None,
        })
    }

    pub fn current(&self) -> ObjectiveState {
        self.current
    }

    pub fn desired(&self) -> ObjectiveState {
        self.pending.map_or(self.current, |request| request.target)
    }

    pub fn expiration_ms(&self) -> Option<u32> {
        self.expiration_ms
    }

    fn queue(
        &mut self,
        target: ObjectiveState,
        interaction: Interaction,
        release: bool,
    ) -> Request {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("secure hold revision exhausted");
        let request = Request {
            revision: self.revision,
            source: self.current,
            target,
            actor: interaction.actor,
            trigger: interaction.trigger,
            release,
        };
        self.pending = Some(request);
        request
    }

    pub fn request(
        &mut self,
        action: Action,
        actor: Actor,
        trigger_index: u8,
        in_range: bool,
    ) -> Result<Request, &'static str> {
        if self.pending.is_some() {
            return Err("secure_hold_transition_pending");
        }
        if actor.team > 1 {
            return Err("secure_hold_invalid_team");
        }
        if action == Action::Release {
            let holding = self
                .holding
                .filter(|h| h.actor == actor)
                .ok_or("secure_hold_inactive_actor")?;
            let target = release_target(self.current).ok_or("secure_hold_no_active_hold")?;
            return Ok(self.queue(target, holding, true));
        }
        if !in_range || trigger_index >= self.rules.trigger_count {
            return Err("secure_hold_invalid_contact");
        }

        let owner = actor.team == self.rules.owner_team;
        let target = match (action, owner, self.current) {
            (Action::Activate, false, Deactivated) => Activating,
            (Action::Activate, false, Retaken) => CancelRetaken,
            (Action::Deactivate, true, Activated | Completed) => {
                if !self.rules.can_deactivate {
                    return Err("secure_hold_deactivation_disabled");
                }
                if self
                    .selected_trigger
                    .is_some_and(|selected| selected != trigger_index)
                {
                    return Err("secure_hold_wrong_selected_trigger");
                }
                if self.current == Activated {
                    Deactivating
                } else if !self.rules.can_retake {
                    return Err("secure_hold_completion_permanent");
                } else {
                    Retaking
                }
            }
            _ => return Err("secure_hold_action_disallowed"),
        };
        Ok(self.queue(
            target,
            Interaction {
                actor,
                trigger: trigger_index,
            },
            false,
        ))
    }

    /// Poll one objective update opportunity against the committed state. When a
    /// hold loses its owning actor, request the same rollback as a release.
    pub fn due(
        &mut self,
        now_ms: u32,
        actor_is_current: impl FnOnce(Actor) -> bool,
    ) -> Option<Request> {
        if self.pending.is_some() {
            return None;
        }
        if let Some(holding) = self.holding
            && !actor_is_current(holding.actor)
        {
            return release_target(self.current).map(|target| self.queue(target, holding, true));
        }
        let (deadline, target) = match self.current {
            Activating => (self.expiration_ms, Activated),
            Activated => (self.expiration_ms, Completed),
            Deactivating => (self.deactivation_ms, Deactivated),
            Retaking => (self.expiration_ms, Retaken),
            Retaken => (self.expiration_ms, Deactivated),
            CancelRetaken => (self.deactivation_ms, Completed),
            _ => return None,
        };
        if now_ms < deadline? {
            return None;
        }
        let interaction = self.holding.or(self.completed_by)?;
        Some(self.queue(target, interaction, false))
    }

    /// Commit the queued transition once. The caller may supply the server-owned
    /// modified hold duration for an entering actor. Absent a modifier, use the
    /// corresponding authored duration. Completion windows use authored time.
    pub fn commit(
        &mut self,
        request: Request,
        now_ms: u32,
        modified_hold_ms: Option<u32>,
    ) -> Result<Entry, &'static str> {
        if self.pending != Some(request) || request.source != self.current {
            return Err("secure_hold_stale_transition");
        }

        let interaction = Interaction {
            actor: request.actor,
            trigger: request.trigger,
        };
        let mut entry_actor = request.actor;
        match request.target {
            Activating | Retaking => {
                self.holding = Some(interaction);
                self.expiration_ms =
                    Some(now_ms.wrapping_add(modified_hold_ms.unwrap_or(self.rules.activate_ms)));
            }
            Deactivating | CancelRetaken => {
                self.holding = Some(interaction);
                self.deactivation_ms =
                    Some(now_ms.wrapping_add(modified_hold_ms.unwrap_or(self.rules.deactivate_ms)));
            }
            Activated | Retaken => {
                if request.release {
                    entry_actor = self
                        .completed_by
                        .map_or(entry_actor, |previous| previous.actor);
                } else {
                    self.completed_by = Some(interaction);
                    self.expiration_ms = Some(now_ms.wrapping_add(self.rules.complete_ms));
                }
                self.holding = None;
                self.deactivation_ms = None;
            }
            Completed | Deactivated => {
                if request.target == Deactivated {
                    self.selected_trigger = None;
                }
                self.holding = None;
                self.completed_by = None;
                self.expiration_ms = None;
                self.deactivation_ms = None;
            }
            _ => return Err("secure_hold_unsupported_entry"),
        }
        if matches!(
            request.target,
            Activating | Retaking | Deactivating | CancelRetaken
        ) {
            self.selected_trigger = Some(interaction.trigger);
        }
        let previous = self.current;
        self.current = request.target;
        self.pending = None;
        Ok(Entry {
            previous,
            current: self.current,
            actor: entry_actor,
            expiration_ms: self.expiration_ms,
            deactivation_ms: self.deactivation_ms,
        })
    }
}

fn release_target(current: ObjectiveState) -> Option<ObjectiveState> {
    match current {
        Activating => Some(Deactivated),
        Retaking => Some(Completed),
        Deactivating => Some(Activated),
        CancelRetaken => Some(Retaken),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ATTACKER: Actor = Actor {
        player: 3,
        generation: 1,
        team: 0,
    };
    const DEFENDER: Actor = Actor {
        player: 4,
        generation: 7,
        team: 1,
    };

    fn rules() -> Rules {
        Rules {
            owner_team: 1,
            trigger_count: 2,
            can_deactivate: true,
            can_retake: true,
            activate_ms: 100,
            complete_ms: 250,
            deactivate_ms: 70,
        }
    }

    fn due(world: &mut SecureHold, now: u32) -> Entry {
        let pending = world.due(now, |_| true).unwrap();
        world.commit(pending, now, None).unwrap()
    }

    #[test]
    fn activation_has_separate_hold_and_completion_entries() {
        let mut world = SecureHold::new(Deactivated, rules()).unwrap();
        let activate = world.request(Action::Activate, ATTACKER, 1, true).unwrap();
        assert_eq!(world.current(), Deactivated);
        assert_eq!(world.desired(), Activating);
        assert_eq!(
            world.commit(activate, 500, Some(30)).unwrap().expiration_ms,
            Some(530)
        );
        assert!(world.due(529, |_| true).is_none());
        assert_eq!(due(&mut world, 530).current, Activated);
        assert_eq!(world.expiration_ms(), Some(780));

        let cancel = world
            .request(Action::Deactivate, DEFENDER, 1, true)
            .unwrap();
        let entry = world.commit(cancel, 600, None).unwrap();
        assert_eq!(entry.current, Deactivating);
        assert_eq!(entry.expiration_ms, Some(780));
        assert_eq!(entry.deactivation_ms, Some(670));

        let release = world
            .request(Action::Release, DEFENDER, u8::MAX, false)
            .unwrap();
        let entry = world.commit(release, 640, None).unwrap();
        assert_eq!(entry.current, Activated);
        assert_eq!(entry.actor, ATTACKER);
        assert_eq!(entry.expiration_ms, Some(780));
        assert_eq!(entry.deactivation_ms, None);
        assert_eq!(due(&mut world, 780).current, Completed);
        assert_eq!(
            world.commit(activate, 800, None),
            Err("secure_hold_stale_transition")
        );
    }

    #[test]
    fn retake_and_cancel_use_distinct_timers_and_restore_expiration() {
        let mut world = SecureHold::new(Completed, rules()).unwrap();
        let retake = world
            .request(Action::Deactivate, DEFENDER, 0, true)
            .unwrap();
        assert_eq!(retake.target(), Retaking);
        assert_eq!(
            world.commit(retake, 300, None).unwrap().expiration_ms,
            Some(400)
        );
        assert_eq!(due(&mut world, 400).current, Retaken);
        assert_eq!(world.expiration_ms(), Some(650));

        let cancel = world.request(Action::Activate, ATTACKER, 0, true).unwrap();
        let entry = world.commit(cancel, 425, None).unwrap();
        assert_eq!(entry.current, CancelRetaken);
        assert_eq!(entry.deactivation_ms, Some(495));
        assert_eq!(entry.expiration_ms, Some(650));

        let release = world
            .request(Action::Release, ATTACKER, u8::MAX, false)
            .unwrap();
        let entry = world.commit(release, 450, None).unwrap();
        assert_eq!(entry.current, Retaken);
        assert_eq!(entry.actor, DEFENDER);
        assert_eq!(entry.expiration_ms, Some(650));
        assert!(world.due(649, |_| true).is_none());
        assert_eq!(due(&mut world, 650).current, Deactivated);
    }

    #[test]
    fn successful_cancel_returns_to_completed_and_stale_retake_rolls_back() {
        let mut world = SecureHold::new(Completed, rules()).unwrap();
        let retake = world
            .request(Action::Deactivate, DEFENDER, 0, true)
            .unwrap();
        world.commit(retake, 100, None).unwrap();
        let replacement = Actor {
            generation: 8,
            ..DEFENDER
        };
        assert_eq!(
            world.request(Action::Release, replacement, 0, false),
            Err("secure_hold_inactive_actor")
        );
        let rollback = world.due(200, |actor| actor == replacement).unwrap();
        assert_eq!(rollback.target(), Completed);
        assert_eq!(world.current(), Retaking);
        world.commit(rollback, 200, None).unwrap();

        let retake = world
            .request(Action::Deactivate, replacement, 0, true)
            .unwrap();
        world.commit(retake, 250, None).unwrap();
        due(&mut world, 350);
        let cancel = world.request(Action::Activate, ATTACKER, 0, true).unwrap();
        world.commit(cancel, 400, Some(45)).unwrap();
        assert!(world.due(444, |_| true).is_none());
        assert_eq!(due(&mut world, 445).current, Completed);
    }

    #[test]
    fn wrong_role_or_trigger_and_disabled_retake_are_rejected() {
        let mut world = SecureHold::new(Deactivated, rules()).unwrap();
        assert_eq!(
            world.request(Action::Activate, DEFENDER, 0, true),
            Err("secure_hold_action_disallowed")
        );
        assert_eq!(
            world.request(Action::Activate, ATTACKER, 2, true),
            Err("secure_hold_invalid_contact")
        );
        let activate = world.request(Action::Activate, ATTACKER, 0, true).unwrap();
        world.commit(activate, 100, None).unwrap();
        due(&mut world, 200);
        assert_eq!(
            world.request(Action::Deactivate, DEFENDER, 1, true),
            Err("secure_hold_wrong_selected_trigger")
        );
        assert_eq!(
            world.request(Action::Deactivate, ATTACKER, 0, true),
            Err("secure_hold_action_disallowed")
        );
        let mut world = SecureHold::new(
            Completed,
            Rules {
                can_deactivate: false,
                ..rules()
            },
        )
        .unwrap();
        assert_eq!(
            world.request(Action::Deactivate, DEFENDER, 0, true),
            Err("secure_hold_deactivation_disabled")
        );
    }

    #[test]
    fn permanent_completion_refuses_retake_but_keeps_window_defence() {
        let rules = Rules {
            can_retake: false,
            ..rules()
        };
        let mut world = SecureHold::new(Deactivated, rules).unwrap();
        let activate = world.request(Action::Activate, ATTACKER, 0, true).unwrap();
        world.commit(activate, 0, None).unwrap();
        assert_eq!(due(&mut world, 100).current, Activated);
        // Inside the completion window the owner may still reverse the hold.
        let defend = world
            .request(Action::Deactivate, DEFENDER, 0, true)
            .unwrap();
        assert_eq!(defend.target(), Deactivating);
        world.commit(defend, 120, None).unwrap();
        assert_eq!(due(&mut world, 190).current, Deactivated);

        let mut world = SecureHold::new(Completed, rules).unwrap();
        assert_eq!(
            world.request(Action::Deactivate, DEFENDER, 0, true),
            Err("secure_hold_completion_permanent")
        );
        assert_eq!(world.current(), Completed);
        assert_eq!(world.due(10_000, |_| true), None);
    }
}
