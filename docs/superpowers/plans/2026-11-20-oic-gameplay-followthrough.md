# OIC Gameplay Follow-through Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans task-by-task. Track completed source and remaining evidence in the task ledger.

**Goal:** Finish the remaining platoon gameplay consumers, implement verified OIC authority and strategic abilities, and continue gameplay TODOs.

**Architecture:** Reuse recorded Engine/world candidates and source-bound neutral catalogs. Company-level strategic state uses authenticated held roles and existing detection, cooldown and spawn consumers.

**Tech Stack:** Rust, serde, Python neutral extraction, offline private retail PPC/Ghidra evidence.

**Spec:** docs/superpowers/specs/2026-11-20-oic-gameplay-followthrough-design.md

## Global Constraints

- Fresh public main 815b971; existing isolated codex/oic-gameplay-followthrough branch.
- Gameplay first; no speculative rules, automatic releases, deployments or game launches.
- Preserve released gitlink f94705e and all unrelated state. No XERJ.
- Debug guides discovery; retail verifies each decision and data field.

## Review Focus

- OIC promotion must not leave stale lower-role authority or duplicate holders.
- A reused slot or other-team request must not acquire leadership, effect credit or a cooldown reset.
- Concurrent strategic effects must remove only their own scoped contribution on expiry.
- Existing and late recipients must see matching company, sensor and spawn deadline state.
- Failed records and queries must preserve the entire prior world, timers and publications.

### Task 1: Acquisition Structural consumers

**Files:** new acquisition_structural module; acquisition_gameplay_engine.rs/config and neutral projector; focused synthetic tests.
**Interfaces:** optional legacy Setup.structural input; round-owned cloned Structural runtime staged within the existing callback candidate.
- [x] Qualify original class effects, sequence collision and FSP dependencies on all three Acquisition maps.
- [x] Write failing gameplay/rollback tests, then connect shared proven Structural core to normal Acquisition entries.
- [x] Reproject source data; verify focused checks and commit source.

### Task 2: Company authority and wire contract

**Files:** leadership_runtime.rs/passives; policy projector and synthetic election tests; qualified company replication module.
**Interfaces:** held OIC identity per actual team; native nomination mask, company leader snapshot and policy capability.
- [x] Qualify role, nomination bit, election scoring/order, removal/refill and native company field types.
- [x] Pin exact verified interfaces in the ledger before implementation.
- [x] Write failing authority and promotion/rollback tests, implement the minimum native hierarchy extension, and run focused checks.

### Task 3: Strategic rules, scheduling and consumers

**Files:** new strategic state/effect modules; neutral strategic projector/tests; existing cooldown/detection/spawn consumer adapters.
**Interfaces:** typed enum and work items derived from Task 2's native contract; external complete source-bound settings.
- [x] Verify enum dispatch, accepted/denied requests, timers, effect stacking/removal and consumer arithmetic from retail.
- [x] Pin effect and consumer APIs in the ledger before code; preserve original per-mode/side registration.
- [x] Write and run meaningful failing effect tests, implement qualified work and consumer adapters, and verify expiry/rollback.

### Task 4: Connected Engine integration

**Files:** active ability runtime/event adapter; company replication; resources/match profile reset/config validation and focused integration tests.
**Interfaces:** recorded strategy candidate, round reset, scoped cooldown refresh, detection and wave consumers.
- [x] Connect authenticated 0249 commands with current company authority and supported mode/side rules.
- [x] Stage effects in native manager order and publish company/detection/deadline state with reliable queue checks.
- [x] Verify wrong-role requests, role replacement, late join, effect expiry and failed event/world records.

### Task 5: Verify, integrate and advance

**Files:** public docs, private TODO and owned native/source receipts.
- [x] Run affected complete Rust/Python suites, strict Clippy/fmt, whitespace and public/private boundary checks.
- [x] Validate real source profiles using offline data; create focused PR(s) and integrate when required checks pass.
- [x] Verify merged/checked tree equality and private checkpoint; keep releases manual.
- [x] Continue the next gameplay TODO with the same constraints and truthful acceptance boundary.

## Verification outcome

The connected implementation passes 677 Rust tests and 214 Python tests, strict
Clippy, formatting and binary/example builds. Review regressions cover overdue
wave preservation, obstructed bot retries and Sweep publication across life
replacement. Offline source-profile checks and source integration are recorded
in the task ledger; native acceptance remains pending and releases stay manual.
