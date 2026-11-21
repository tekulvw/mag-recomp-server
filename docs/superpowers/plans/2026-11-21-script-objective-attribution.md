# Script Objective Attribution Implementation Plan

> **For agentic workers:** Execute the authorized parallel Sol native/core tasks and inline Engine integration; use test-first focused checks before integration.

**Goal:** Enable only proved script-driven objective rewards with correct current actor provenance.

**Architecture:** Distinct direct desired setter and ChangeState APIs drive class-specific physical entries. Callback context retains the source objective and resolves its mutable current actor per invocation effect. Existing owned candidates atomically record rewards, counters and lifecycle changes.

**Tech Stack:** Rust gameplay and Python private native/resource qualification.

**Spec:** `docs/superpowers/specs/2026-11-21-script-objective-attribution-design.md`

## Global Constraints

- Source integration only; manual releases after human live-match review.
- Original files/data/evidence remain private; public tests are synthetic.
- Preserve unrelated services, accounts, indices and the released submodule pin.
- No XERJ or game/deployment work.

## Review Focus

- A same-current ChangeState must not cancel an earlier pending target.
- Nil/unresolved callers must preserve credit latches while retaining the correct raw caller field.
- Delayed and successive Original effects must read current source-object actor state, not a frozen player.
- Intermediate entries must not invent planting/retake XP; legitimate completion and defuse callbacks still count.
- Unselectable targets and cyclic scripts must not spin or partially commit.

### Task 1: Qualify and implement class request/entry logic

**Files:** `objective_control.rs`, focused scripted state tests.
**Interfaces:** `stage_native_target(index,target)`, ChangeState `stage_native_state_with_caller(index,target,raw_slot,actor)`, `script_actor(index)` and `has_script_transition(index)`.

- [x] Reproduce Structural/Secure selectors, latch writes, physical entry callbacks and original caller userdata privately.
- [x] Write failing state/actor tests, implement minimal class-specific behavior, pass focused tests.

### Task 2: Wire authored caller and candidate consumers

**Files:** `domination_callbacks.rs`, `domination_gameplay_engine.rs`, `domination_gameplay_provider.rs`, `acquisition_structural.rs`, connected synthetic tests.
**Interfaces:** Context retains `caller_object_id`; each Original effect reads the source objective actor then resolves its current character separately. Consumers process every actual entry with no-progress and 256-transition bounds.

- [x] Reproduce immediate/delayed/current-field tests and rollback/cycle failures.
- [x] Implement direct/ChangeState consumers, qualified rewards and bounds; pass connected tests.

### Task 3: Verify and integrate

- [x] Run affected Rust, strict Clippy/fmt/build, Python and public-boundary checks.
- [ ] Resolve concrete review findings with failing regressions, pass required CI and verify the exact merged source tree.
- [ ] Push the private TODO/evidence checkpoint without moving the released gitlink or cutting a release.
