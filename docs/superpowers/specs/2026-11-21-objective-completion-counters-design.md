# Objective completion counters

Implement the remaining supported retail zero-XP objective counters without adding account XP aliases: subtype repair completion (43/45/47/49/51/53/55/57), type-9 linked vehicle capture (58), and exact tagged-objective completion for current squad leaders (41). Correct rebuilt Structural defuse to the existing ChargeDefused reward rather than treating it as a repair completion. Original executable/resource evidence stays private; public source and tests use synthetic inputs.

## Design

Use existing owned character-effect candidates to stage gameplay counters and StatUpdate messages together with objective state, then record before installing. Counters require qualified native callback provenance; replicated actor fields or forced target changes alone do not establish credit. No new Horizon persistence schema, receipt aliases, XP, FRAGO points, or capability flags are needed for these zero-XP counters. Existing match participation preserves counters on same-character rejoin; normal round reset clears them.

Repaired from Repairing counts the supported subtype once for the currently allocated defender latch. Repaired from Deactivating instead awards existing ChargeDefused, with no repair count or primary-objective XP. Preserve the native owner repair latch even for a health-ceiling packet that adds no contribution/work credit. Qualified class callbacks run after common/FSP effects and before script/assignment changes.

Vehicle capture freezes the human reservation/world/slot/round/authenticated-account identity independently of optional XP metadata at admission. It requires an ordinary hold-completion witness from ObjectiveWorld, the matching pending request version and actor, actual Completed entry and a validated type-9 FSP. Emit after the FSP effects succeed and before the controller script and assignment refresh. Delivery and generic forced/script transitions do not inherit this witness.

Tagged completion uses exact objective family/slot identity, not primary assignment or range. Enumerate the credited actor’s side’s 16 current squads, using current effective tags and their currently allocated published leader. Missing/unallocated leaders receive nothing; no new life or nomination gate is added at settlement. Callback-specific predicates and ordering are verified against retail before enabling paths.

## Scope and alternatives

Direct recorded gameplay counters reuse existing statistics and rollback state. Creating zero-value XP receipts would add persistent compatibility and permission requirements without XP semantics. Updating counters from final state snapshots would lose callback provenance and native ordering. Therefore use the existing callback/candidate path. Generic script attribution remains deferred unless independently proved. No game launch, live service/store, deployment, release, native SDK or released-pin changes.

## Acceptance

Connected synthetic tests prove subtype/old-state selection, rebuilt defuse XP, current last repairer including ceiling-only latch, callback cancellation/replacement, exact tag and side/leader selection, repeated ticks, record failure/retry, same-character rejoin and round reset. Retail byte/resource checkers reproduce privately. Focused/full affected Rust checks, Python extraction checks, formatting, strict Clippy, build, public boundary and required PR CI pass before source integration. Live rendered stats/results remain human live-match acceptance.
