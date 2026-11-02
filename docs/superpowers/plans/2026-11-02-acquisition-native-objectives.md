# Acquisition Native Objective Implementation Plan

**Goal:** Close native controller dependencies and integrate the supported
Acquisition objective catalog/replication path without guessing gameplay effects.
**Architecture:** Reuse ObjectiveCatalog/ObjectiveWorld and the source controller
planner. Independent original-native analysis supplies explicit qualified inputs.
**Spec:** `docs/superpowers/specs/2026-11-02-acquisition-native-objectives-design.md`.

- [x] Reproduce shared objective family/factory allocation and network index joins.
- [x] Bind per-map callbacks/arguments/delay and immediate DamageVolume behavior.
- [x] Write red tests for16squads, source indices and explicit empty assignments.
- [x] Extend catalog validation and verify mode0 behavior remains unchanged.
- [x] Add qualified initial source projection and actual catalog/registry/engine checks.
- [ ] Full callback application requires qualified immediate overlap/damage effects;
  do not partially apply lethal commands or label pre-start states as gameplay.
- [ ] Verify affected Rust/Python checks, fmt/Clippy/public scan and public CI.
- [ ] Merge source; checkpoint private evidence/TODO while preserving release pin.

Review focus: current versus target state, all objective families in pool order,
no capture claims from source models, ordered duplicate squad entries, atomic
command effects, native volume overlaps and bounded original/resource scope.
