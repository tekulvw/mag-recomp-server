# Objective XP and contextual FRAGO rewards

This source change connects the six existing objective reward kinds to the source-owned Acquisition and Domination paths. The legacy objective driver already settled its own capture and demolition rewards; the newer mode drivers bypassed that queue and native-round completion only emitted match results. Native class callbacks now stage their positive action rewards inside the existing character-effect transaction. Acquisition also drains its capture queue before the ensuing controller effects, preserving activation before securing when one late tick crosses both deadlines.

## Qualified callbacks

These are entry callbacks, not final-snapshot comparisons. Authored category is independent of the Structural virtual type.

| Class | Previous → entered state | Positive existing reward kinds |
| --- | --- | --- |
| Secure and Hold | Activating → Activated | Primary activation, category 0 or 1 |
| Secure and Hold | Activated → Completed | Primary secured, category 0 or 1 |
| Secure and Hold | Retaking → Retaken | Primary activation, category 0 or 1 |
| Secure and Hold | CancelRetaken → Completed | Primary activation, category 0 or 1 |
| Secure and Hold | Deactivating → Deactivated | Primary activation, category 0 or 1 |
| Secure and Hold | Retaken → Deactivated | Primary secured, category 0 or 1 |
| Structural | Activating → Activated | Charge planted, followed by primary activation when category ≤1 |
| Structural | Deactivating → Deactivated | Charge defused, followed by primary activation when category ≤1 |
| Structural | Activated → Completed | Primary secured, category 0 or 1 |

Rollback to Activated does not re-award planting. A rebuilt structure defused into Repaired earns ChargeDefused; the follow-on [completion-counter qualification](objective-completion-counters.md) corrects the initial shared-callback interpretation. Only Repairing → Repaired selects a zero-XP subtype repair counter. Completion while a defuse is underway does not award the primary secure increment. Independent subtype destruction counters and their positive XP are separate from the six kinds in this change.

The native callback resolves the current allocated character at its latched player slot. Gameplay requires the current admission to have claimed and entered the world; it does not require a living character or the initiator's old life generation. Missing actors and account-less bots create no account reward. Acquisition's existing delayed capture queue retains its full reservation, world, player, account, character and round identity fence.

`NativeEntry.reward_actor` carries qualified ordinary callback attribution separately from the replicated wire actor. Precision's desired-Completed cascade skips its intermediate planting callback; its final attacker latch depends on source-side resolution and intervening entries. Precision and destroyed-trigger latches are now represented by the [extended reward implementation](extended-objective-rewards.md); generic script attribution now follows the [qualified request and physical-entry paths](script-objective-attribution.md). The replicated wire actor is never a fallback for missing callback provenance.

Common class/FSP effects precede settlement; blasts, authored scripts and later squad reassignment follow. Objective callbacks supply the actual objective as context. Current actor position, current squad assignment and the staged Ground/helicopter state are used for FRAGO range. The frozen optional `frago:true` fact survives movement, reassignment and retransmission.

## Match end and capacity

Before each character's match-result receipt, both owned modes now traverse that character's authored squad objective vector in order, retaining repeated entries. Category 0 contributes primary completion; categories 2 and 3 contribute secondary completion. Attackers qualify for committed Completed/Locked; defenders qualify for committed Inactive/Deactivated/Repaired. A desired script request alone is not a completed state. Each row supplies its own objective context, rather than sharing the outcome-triggering objective.

Ordinary pending rewards retain their 256-entry budget. A separate bounded terminal reserve accommodates 64 represented humans, each with at most 128 authored vector rows and one result. The entire transition freezes and records before installation, then drains through the existing 32-reward wire batches. ACK history remains 256 entries. The reserve is a worker bound; Horizon's existing finite durable receipt limit still applies. It does not promise persistence for a schema-maximum synthetic match or an already full ledger.

## Bridge compatibility and settlement

Hello advertises `frago_objective_bonus_v1` independently of kill and support capabilities. Horizon emits optional `frago_objective_bonus_enabled` only for a negotiating peer, and enables it only with the retail progression catalog and valid authored bonus coefficient. Old Horizon leaves new gameplay at ordinary objective XP; new Horizon omits the new field for old gameplay. A disabled admission cannot submit contextual objective rewards.

All six kinds require one unit. Horizon retains ordinary whole-XP calculation and its shared fractional carry, then computes the authored FRAGO bonus from positive whole XP. XP, carry, accumulated bonus/stat 3, account progression and the immutable receipt/ACK stay in the same store transaction. A qualified zero bonus still requires the immutable bonus snapshot. Fresh rewards require current rules; exact historical replay returns its original acknowledgement before individual current-rule checks. The existing global no-catalog policy is unchanged.

The saved ledger remains schema 3. An older Horizon reader rejects contextual objective receipts newly admitted by this version; preserve receipt history when rolling back binaries.

## Preparing private data

`tools/extraction/project-retail-objective-rewards.py` layers the existing authored Category/SubType classification onto a final source-gameplay objective catalog. It checks the original retail image identity, exact metadata digest, mode/map identity, full token/slot inventory and runtime class compatibility. Secure and Hold, Structural and Acquisition Arrival entries receive classification; Arrival remains without a native action class and contributes only through its proved consumers, including endgame. Existing FRAGO geometry and every other field are preserved. Conflicting preexisting classification, malformed enums and ambiguous overrides are rejected.

```sh
python3 tools/extraction/project-retail-objective-rewards.py \
  --metadata /private-inputs/authored-maps.json \
  --objectives /private-inputs/gameplay-objectives.json \
  --native-elf /private-inputs/original-retail.elf \
  --output /private-outputs/objectives-with-rewards.json
```

Rerun with `--check` for exact-byte reproduction. Use a new output outside the checkout and update only an owned isolated profile's source-bound objective pins. Extraction does not activate profiles, deploy services, edit accounts or publish original data.

## Acceptance boundary and next sources

Verification uses synthetic Rust/Horizon/Python tests and offline original-retail qualification. No live match, deployment or release is part of this change. Structural subtype destruction and Acquisition delivery now use the [additional typed reward group and qualified callback latches](extended-objective-rewards.md). Detonation counters accompany destruction settlement. The [zero-XP tagged, vehicle-capture and repair-completion paths](objective-completion-counters.md) are recorded by the gameplay callback independently of XP admission. Rendered XP and final-results behavior still require live-match review before a human cuts a release.
