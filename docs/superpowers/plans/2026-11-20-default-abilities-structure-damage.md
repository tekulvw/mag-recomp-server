# Default Abilities and Structure Damage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Enable supported squad abilities by default and execute source-qualified strike damage to structures through recorded world lifecycles.

**Architecture:** Default input resolution belongs to worker/profile loading. Raw collider identity and damage receivers determine structure admission; the existing combat and provider transaction owns staging and publication.

**Tech Stack:** Rust, Python, original retail ELF/archive witnesses, focused Ghidra when necessary.

**Spec:** docs/superpowers/specs/2026-11-20-default-abilities-structure-damage-design.md

## Global Constraints

- No XERJ; use rg and direct reads. DEV names may locate but retail must verify decisions/data.
- Original files, decompilation, populated catalogs and raw receipts remain private.
- Preserve manual releases, released gitlink, accounts, services, native sessions and unrelated source.
- No game/deployment work; offline extraction and bounded source tests only.

## Review Focus

- Multiple maps sharing a provider directory must resolve different deterministic defaults.
- Missing default inputs must not silently disable the mechanic or grant development leadership.
- Raw collider class IDs must not be confused with outgoing network pool IDs.
- Strike destruction must preserve later occupant/collision/objective lifecycle work.
- Recording failure and invoker replacement must not spend another strike or misattribute damage/rewards.

### Task 1: Default supported source-gameplay abilities

**Files:** gameplay/src/main.rs, gameplay/src/match_profiles.rs, active_ability_config.rs where needed, associated synthetic tests and operator docs.

**Interfaces:** Per-profile and fixed-worker loading still produce active_ability_engine::Setup; explicit configuration paths remain overrides.

- [ ] Add failing tests for deterministic per-map default resolution, missing/mismatched data, override, native-role requirement and unchanged mode0/shell configurations.
- [ ] Implement automatic required loading for concrete mode1/2 providers.
- [ ] Generate matching private default sidecars/manifests and validate all twelve source profiles.
- [ ] Run affected checks and report the exact default convention.

### Task 2: Qualify retail structure receivers and inputs

**Files:** Private server-structure-damage-20261120/native and hospitals research, with neutral public projector changes only when needed.

**Interfaces:** Supply original-byte-backed target class/body identity, damage metadata, armor/guard semantics and deferred lifecycle ordering to Task3.

- [ ] Trace Cluster instantaneous collector and accepted/skipped body families.
- [ ] Bind raw collider user-data to actual objective/hospital/other structure creation, separate from network IDs.
- [ ] Qualify normal receiver, authored data and any immunity before exposing a target in the server.
- [ ] Retain reproducible original witnesses and extract external inputs with source hashes.

### Task 3: Connected structural damage

**Files:** Focused structure damage module/tests, domination_structural_world.rs, structures.rs, active_ability_engine.rs and world provider hooks as required by Task2.

**Interfaces:** Extend collected work/commit output with qualified structure targets or doses; providers apply deferred lifecycle in their existing owning transaction.

- [ ] Add a failing timed strike regression against a qualified synthetic target and relevant immunity/cover test.
- [ ] Route the original metadata through the appropriate shared damage model and stage its native lifecycle consequences.
- [ ] Cover friendly fire, cached attribution, duplicate/dead state, destruction/repair and failed-record/query rollback.
- [ ] Verify current source-map inputs and normal Engine/provider integration without game launches.

### Task 4: Verify and integrate source

- [ ] Run affected tests, full Rust/Python suites, strict Clippy/fmt and public-source boundary scan.
- [ ] Review concrete core gameplay risks; fix demonstrated issues and preserve fidelity limits.
- [ ] Reconcile fetched main, open focused source PR(s), wait for required CI and integrate exact checked heads.
- [ ] Check merged trees, record private evidence/TODO on private main and retain released pin/manual release PR.
