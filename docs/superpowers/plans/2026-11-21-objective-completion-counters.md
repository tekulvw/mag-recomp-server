# Objective Completion Counters Implementation Plan

> **For agentic workers:** Use the already authorized parallel Sol agents for independent native/latch/capture tasks and inline execution for shared callback integration.

**Goal:** Record qualified zero-XP objective counters and correct rebuilt defuse XP.

**Architecture:** Existing character-effect candidates stage counters/messages with objective transitions; dedicated counter helpers select recipients from current callback context. Horizon XP receipts retain their existing schema.

**Tech Stack:** Rust gameplay, Python source-bound extraction/checkers, existing C# regression suite.

**Spec:** `docs/superpowers/specs/2026-11-21-objective-completion-counters-design.md`

## Global Constraints

- Public source and synthetic tests only; original retail bytes and receipts stay private.
- No XERJ, game launch, live service/store, deployment, release or released-pin change.
- Preserve unrelated changes and native indices. Source integration follows passing checks.

## Review Focus

- A rebuilt defuse must earn ChargeDefused but no subtype repair counter.
- Last repairer ceiling-only packets must change ownership without extra work/XP.
- Forced and superseded capture requests must not invent vehicle counts.
- Tags use exact family/slot, current same-side squads and allocated leader, without primary/range fallback.
- Recorder failures must restore counters, messages, latches and transitions for retry.

### Task 1: Native qualification and repair actor ownership

**Files:** `objective_control.rs`, `domination_repair_runtime.rs`, Acquisition/Domination repair transaction sections and synthetic repair tests.
**Interfaces:** Preserve public repair behavior; add explicit latch-only disposition where required. NativeEntry.reward_actor conveys the qualified current defender on Repairing→Repaired.

- [x] Reproduce original repair/defuse/tag/capture byte and resource checks privately.
- [x] Write and run failing tests for old5→6 actor, last/ceiling repairer and recorder rollback.
- [x] Implement the latch and recorded latch-only path; run focused repair tests.

### Task 2: Counter and callback integration

**Files:** new `gameplay/src/objective_counters.rs` and tests, `objective_rewards.rs`, `acquisition_structural.rs`, `domination_gameplay_provider.rs`, `acquisition_gameplay_engine.rs`.
**Interfaces:** `Engine::stage_objective_counters(player, object_index, counters, tagged, assignments)` returns audit/messages; mutations live within the existing owned character candidate.

- [x] Write failing tests for all eight repair counters, rebuilt-defuse reward, exact tag/side/current leader, no capability/account dependency, saturating values and rollback.
- [x] Implement direct statistics publication and connect qualified native callbacks.
- [x] Run focused tests and check unchanged reward replay/compatibility behavior.

### Task 3: Acquisition capture witness

**Files:** `acquisition_gameplay.rs`, ordinary ObjectiveWorld capture queue, Acquisition tick and connected tests.
**Interfaces:** `apply_entry_with_capture` preserves the existing `apply_entry` default and emits a capture effect after FSP effects; explicit queue witness binds target version and actor.

- [x] Write failing tests for actual ordinary type9 capture, script/delivery negatives, supersession and record retry.
- [x] Implement the witness and pre-controller effect; run connected capture tests.

### Task 4: Verify and integrate

- [x] Run affected full Rust, strict Clippy/fmt/build, Python and public boundary checks.
- [ ] Check the final source diff, required CI, and merge only the source PR.
- [ ] Verify merged tree equality and push private TODO/evidence checkpoint without moving the released gitlink.
