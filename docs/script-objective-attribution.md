# Scripted objective attribution

Authored objective requests now use the same qualified physical state callbacks as ordinary gameplay. A requested target alone grants no XP or counter. Existing Horizon kinds, capability negotiation, fractional carry and immutable settlement remain unchanged.

## Requests and actor fields

Domination Mission_SetObjectiveState uses ChangeState. A target equal to current state leaves a different pending request untouched. An admitted request writes its raw caller slot and updates only the appropriate side latch when that character currently resolves. Nil or an unresolved slot preserves both credit latches. Duplicate pending requests can still update caller/side ownership.

Direct Enable/Disable/Lock and MissionStart setters carry no caller, and can replace a different pending target even when the new target equals current. Acquisition stages all native MissionStart targets before normal updates, preserving later hook and ability requests.

The source objective's current script actor, its attacker/defender latches and the network actor are separate fields. Physical entries update their own actor and clearing rules. Precision carries the cached raw caller independently of its wire source; a missing wire source is not permission to reuse a stale script actor.

## Physical transitions

Structural and Secure requests follow their own observed selector paths through intermediate entries. Each entry retains the existing common/FSP, stats, blast, script, assignment and sequence order. Intermediate Activated and Retaken entries under a different desired target do not generate planting/activation notifications. Actual Completed and Deactivated callbacks use their prior physical state and current side latch. Existing authored classification still governs the reward kind and amount.

Secure Completed publishes health 0 and Deactivated health 100 through the same helper for ordinary and scripted entries. Constructor snapshots retain their existing pre-entry initialization. A same-state direct re-entry retains the wire state-change timestamp. Structural Locked has no selectable route in the qualified class, and none of the represented authored Structural commands requests it; an unselectable request remains pending without spinning. Consumers bound continuing transitions at 256 per objective within the owned candidate.

## Original caller and delayed invocations

The script scheduler retains the source objective's packed identity. Each Original setter resolves that objective's current script actor at execution, including five-second delayed callbacks. Successive effects can therefore observe a caller-field change made by an earlier effect in the same invocation. A wire actor or a player captured at the original event time is not a substitute.

Acquisition's immediate AA hooks read the current source objective actor; MotorPool setters pass nil. Native targets receive those requests at invocation, rather than delaying team/actor resolution until the target's later update. Passive marker consumers retain their qualified stable-state scope.

The represented Domination delayed resets can reach existing primary-objective rewards only if their eventual physical state edge and defender latch qualify. The reviewed Acquisition controller and passive-marker branches add no separate positive reward. An eligible callback can still receive the existing tag/FRAGO handling; this change does not award XP for each script call.

## Atomicity and limits

All target requests, actor fields, callbacks, rewards, counters and publications stay inside the existing record-before-install candidates. Recording, outbox or effect failure installs none of the candidate. Cyclic callback graphs stop at the transition bound; an unchanged tick does not invent a fresh transition.

Original native and authored-resource qualification is retained privately for all six represented maps. The four unresolved hooks in each of Elk Bay and Alyeska remain explicit and unchanged. This source verification does not establish live match trajectories, rendered HUD/results or a release decision. Releases remain manual after human live-match review.
