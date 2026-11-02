# Acquisition Effect Qualification Implementation Plan

> Execute inline with superpowers:executing-plans; native investigations run in
> independent scratch directories. User standing directions waive routine proposal
> approval and review ceremonies.

**Goal:** Build the authoritative effect backend from qualified native behavior,
keeping unsupported effects explicit until their real implementation exists.

**Architecture:** Extend the current Rust implementation with bounded native
state/assignment/damage primitives; connect only complete authoritative effects
to the atomic adapter. Evidence and extracted original values stay private.

**Tech Stack:** Rust, existing serde/Cargo tests, private Python PPC/DWARF helpers.

**Spec:** ../specs/2026-11-02-acquisition-effect-qualification-design.md

## Global constraints

No live launches/deployments. Manual releases only. Preserve released pin and
unrelated native indices; public source/invented fixtures only. Use fresh main.

## Review focus

Current/target separation; ordered lists and null entries; no-valid primary versus
independent FRAGO; manual tags; incomplete collision/death providers cannot pass.

### Task 1: Native qualification

- [ ] Reproduce remaining overlap, lethal/death and entry/assignment behavior
  using whole-function native bridges where possible.
- [ ] Record exact qualified scope and concrete unresolved requirements.

### Task 2: Qualified objective state and assignment behavior

Files: gameplay/src/acquisition_objective_runtime.rs,
gameplay/tests/acquisition_objective_runtime.rs, gameplay/src/lib.rs.

- [x] Add tests for current0 Enable, current!=0 Disable, current!=9 Lock,
  unchanged pending targets, Take/Hold owner-team/state truth tables, first-valid
  primary and FRAGO, missing spawn/list/mode gate, manual tags and invalid IDs.
- [x] Run failing tests before implementation.
- [x] Implement only source-bound admitted behavior with explicit raw state IDs.
- [x] Run focused tests, fmt and strict Clippy.

### Task 3: Additional effect implementation

- [ ] Implement exact native geometry/lethal constructors or handlers supported
  by Task1; add critical failing tests before code.
- [ ] If complete body coverage or consequences remain unresolved, retain errors
  and document that the concrete backend is not yet installable; do not add a
  dummy live backend or infer compatibility from synthetic worlds.

### Task 4: Integration and private checkpoint

- [ ] Run affected/full critical Rust checks and public source boundary scan.
- [ ] Fetch/reconcile, create focused public source PR and integrate on green CI.
- [ ] Track proof tools/receipts and update private TODO on main using exact paths.
- [ ] Recheck unchanged released pin/composition; keep releases manual.

## Execution findings

Bounded native investigations reproduced assignment, overlap convention/filter,
damage and previous-state bookkeeping proofs. Complete live collision-body
coverage and lifecycle/entry/tag capabilities remain unresolved, so Task1/full
backend completion stays open. Task3 implements scalar geometry/current-filter
and precise lethal request metadata only; no placeholder backend was installed.
Critical tests passed after their missing-module/function failures were observed.
