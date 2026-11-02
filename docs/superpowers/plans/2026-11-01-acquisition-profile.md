# Acquisition Profile Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Project Arulkum Acquisition safely, then admit a bounded retail match
with two platoons on each side.

**Architecture:** Extend the existing original-resource projector and disk spawn
schema. Keep the64-player transport bound and distribute32 players per team over
eight squads of four. Reuse existing typed replication and team-wave clocks.

**Tech Stack:** Python stdlib, Rust, Horizon C#.

**Spec:** `docs/superpowers/specs/2026-11-01-acquisition-profile-design.md`

**Scope update:** The human subsequently requires vehicle spawning. Basic-only
projection/default fallbacks are a foundation, not completed Acquisition entry.
Research the authored vehicle links and native creation/insertion contract while
preparing bounded admission; implement supported vehicle creation before claiming
mode acceptance. Preserve unknown identities/codecs as explicit gaps.

## Global Constraints

- Public replacement source and invented fixtures only; original data private.
- Preserve released public pin, accounts, services and manual releases.
- Never enable unsupported vehicle/air/drop spawn points.
- Preserve existing Suppression/Sabotage projection bytes.

## Review Focus

- Unsupported active points must remain unavailable.
- Vehicle insertion squads need a unique explicit Basic alternate fallback.
- Attacker selection must be explicit and cannot equal the defender.
- Both platoons must preserve native squad addresses without increasing capacity.
- Mode admission must remain synchronized and retail-only.

### Task1: Project the Acquisition profile

**Files:** `tools/extraction/build-retail-spawn-catalog.py`,
`tests/test_acquisition_projection.py`; private original reproduction/catalog and
readiness/TODO checkpoint in the private repository.

**Interfaces:** Extend `build(..., attacking_faction=None)` with explicit
Acquisition attacker policy; emit the existing mag_spawn_catalog v1 schema.

- [ ] Write synthetic projection tests for retained unsupported points, two
  platoons, default fallbacks, explicit attacker and unchanged mode policy.
- [ ] Run tests and confirm missing Acquisition behavior fails.
- [ ] Implement selected GameType128 rules/map handling and projection.
- [ ] Run Python checks/public guard; reproduce the original private profile
  from explicit original archives and verify legacy projections unchanged.
- [ ] Integrate the focused public PR after CI; checkpoint private evidence.

### Task2: Bounded admission and second-platoon spawning

**Files:** `gameplay/src/admission.rs`, affected squad/replication tests,
`horizon/Server.Medius/Medius/LocalDmeHost.cs` and its synthetic checks.

**Interfaces:** Retail mode1 uses the configured maximum64; prototype stays8.
Existing SquadAddress indices0..31 and configured wave policy remain authoritative.

- [ ] Write critical mode/capacity and second-platoon lifecycle tests, run red.
- [ ] Fix only demonstrated admission/replication assumptions.
- [ ] Run affected Rust/Horizon suites, formatting, Clippy and public guard.
- [ ] Integrate after checks and retain exact source/private evidence identities.

### Task3: Required vehicle spawning

- [ ] Map authored Acquisition vehicle-spawn links to the maintained runtime
  representation and distinguish vehicle creation from passenger spawn selection.
- [ ] Establish native creation/pool/type and vehicle-backed insertion contracts.
- [ ] Add synthetic regressions for creation, assigned squad access and lifecycle
  cleanup, observe red, then implement the supported typed paths.
- [ ] Integrate after focused checks; keep native correctness acceptance separate
  and do not claim entry complete with only Basic-alternate fallbacks.
