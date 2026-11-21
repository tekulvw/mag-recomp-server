# FRAGO Kill Bonus Implementation Plan

> **For agentic workers:** Parallel Sol6.2 native proof and Horizon ownership, root Rust/projection implementation, one final focused integration review. User fast-loop autonomy supersedes proposal/per-task approval gates.

**Goal:** Award exact contextual FRAGO bonus once on qualified existing enemy kills.
**Architecture:** Source-bound objective geometry and current assignment/actor contexts freeze an optional private fact; Horizon atomically calculates bonus and XP; Rust consumes immutable cumulative stat3 ACKs.
**Tech Stack:** Rust, C# Horizon, Python neutral projection and private exact-retail checker.
**Spec:** docs/superpowers/specs/2026-11-20-frago-kill-bonuses-design.md

## Global Constraints
- Fresh baseline861af10/tree80accf24; private028bedd and released f94705e remain preserved. Retain native dirty submodule, unrelated work, raw evidence and accounts.
- Only qualified EnemyKilled unit1 contexts; no invented objective state/team/HP predicates, separate stat41 and non-kill families.
- Loaded complete source geometry and admission capability required. Legacy inputs and no context remain XP-only.
- Source f32 XYZ inclusive arithmetic; actor OR victim, effective tag OR distinct primary, current per-event authority and frozen receipt.
- Base XP/carry first, positive whole-XP bonus truncation, positive bonus stat3 and BattleXP; immutable ACK/tally atomic record-before-install.
- Public source/synthetic only; no XERJ/game/services/accounts/deployment/release/pin changes.

## Review Focus
- Explicit zero radius/default distinction, 3D equality, different tag/primary and victim-only range.
- Allocated inactive/destroyed targets still resolve; missing/wrong family/world/source and invalid current geometry do not.
- Friendly/self/finishing and unqualified catalogs do not acquire a bonus; human and bot weapon-independent kill receipts retain identity.
- Changed context cannot replay; delayed ACK cannot regress stat3 or contaminate a new round or replacement character.
- Store failure, record failure, carry, overflow and qualified zero preserve atomicity; source projections retain all existing catalog fields.

### Task1: Native and Horizon contract
**Files:** private proof only native worker; horizon catalog/store/bridge/admission and FragoBonusChecks synthetic tests store worker.
**Interfaces:** optional reward frago:true only; optional private admission frago_bonus_enabled bool; optional immutable ACK match_frago_points:u32.
- [x] Prove retail predicate/getters/kill context, positive stat eligibility, arithmetic, default radius and current-pose limits; retain reproducible checker/receipt.
- [x] Observe Horizon RED, implement coefficient/capability, atomic tally/receipt/ACK and compatibility checks; pinned SDK full checks GREEN, commit exact owned paths.

### Task2: Source geometry, capture and ACK
**Files:** new gameplay/src/frago.rs and frago_tests.rs; objective_catalog.rs, lib.rs, progression.rs, events/statistics.rs, events/mod.rs, combat.rs and update_actions.rs; synthetic fixtures. New tools/extraction/project-retail-frago-rules.py and tests/test_frago_rules.py.
**Interfaces:** ObjectiveCatalog.frago:Option<frago::Rules>; Engine::frago_kill_reward(event,killer,context_position,Frame)->RewardEvent; Rules::validate(source,object_indices,map_name,game_type), source projections bound to selected catalog; Request::ProgressionCommitted optional match_frago_points.
- [x] Write connected failing kill/capture and ACK tests, plus literal projection tests for authored/default/zero/hash/inventory.
- [x] Implement typed complete geometry loading and maintained projection, exact source predicate with current inventory and primary/tag OR.
- [x] Attach fact at all source-qualified human/bot kill creation points before record; current stable IDs and friendly/replacement checks unchanged.
- [x] Commit stat3 snapshot after ACK record with max/current-round identity rules; test replay/rollback/late reconnect and zero/legacy cases.

### Task3: Integration
- [x] Run focused/full Rust, Python, Horizon, Clippy/fmt/build/public boundary; reproduce actual map projections privately. One final focused review; fix concrete findings with RED/GREEN.
- [ ] Integrate focused public source PR after required CI; verify exact merged tree and private main checkpoint/TODO, preserving released pin/manual releases.
