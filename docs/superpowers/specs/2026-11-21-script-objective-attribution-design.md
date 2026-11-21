# Script objective attribution

Connect generic authored Acquisition and Domination objective requests to the existing qualified reward/counter callbacks using original-retail actor, desired-state and entry semantics. Preserve manual releases, source-only integration, private original inputs, current SDK indices and unrelated work. No XERJ, game run, live service/store, deployment or release operation.

## Request and entry behavior

Domination Mission_SetObjectiveState performs ChangeState. A request equal to current state leaves the existing desired request and actor fields untouched. An admitted request installs/reuses a desired target, writes the raw caller slot and, if the supplied character currently resolves on the appropriate side, updates only the attacker or defender latch. Nil or unresolved callers preserve those latches. Acquisition Enable/Disable/Lock and mission startup use the direct desired setter and do not manufacture a caller. A direct setter can replace a different pending target even when its new target equals current.

Use the proved class-specific selectors to enter physical states before the requested stable target. Process each actual entry's common/FSP effects, qualified stats, blast, script and assignments in existing order before continuing. Intermediate Activated/Retaken entries under a different desired target do not notify planting/activation stats. Completed and Deactivated callbacks qualify their actual prior physical states. Entry latch clearing and the source objective's current script actor are separate from the wire actor. Unselectable class targets remain pending without a busy loop. Bound repeated callback transitions within the owned transaction.

## Authored caller

The script scheduler retains the source objective's packed identity. Mission_SetObjectiveState resolves Original userdata to the Objective facet and reads its current actor field at invocation, including delayed invocations. Store the original source-object identity in callback context and resolve its current actor separately for every effect. Preserve a raw actor slot even when character resolution fails; only a currently allocated character supplies team qualification for credit-latch writes. Do not substitute a wire actor, event-time player snapshot, account identity or alive check. Existing account XP settlement still resolves the qualified current callback recipient.

## Scope and implementation choices

Reuse existing catalog fields, callback queue and record-before-install candidates. No new Horizon aliases or balance data. A blanket script reward exception loses legitimate native credit; applying rewards directly from target/wire changes invents callbacks. Therefore implement the observed desired/entry sequence and explicit latches. Keep unresolved authored hooks and unsupported native operations explicit. Runtime rendering and broader fidelity remain live acceptance work.

## Acceptance

Synthetic regressions cover request no-op/replacement/duplicate semantics, each represented selector, intermediate notification suppression, actor side/unresolved behavior, mutable Original caller at immediate and delayed invocation, script rewards/tag counters through actual callbacks, cycle bounds, recorder rollback and repeat ticks. Reproduce native byte/resource witnesses privately, run affected Rust/Python/public-boundary checks and required CI, then integrate only the checked source tree and record the private TODO checkpoint.
