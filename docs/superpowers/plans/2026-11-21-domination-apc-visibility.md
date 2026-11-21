# Domination APC passenger visibility implementation plan

> Continue the user-approved autonomous gameplay-first TODO loop. Parallel Sol6.2 implements Rust qualification and connected tests; root owns extraction and original-source verification. Routine design-approval prompts are superseded by the user's instruction to keep advancing.

**Goal:** Prevent already supported APC passenger spawning from interrupting Domination Structural periodic area processing.
**Architecture:** Reuse the existing authored physical-seat visibility projection for Domination's qualified vehicle source scope. Require complete admitted-seat coverage at startup only where a bound Structural objective has enabled periodic area damage; the existing area transaction continues to skip hidden occupants and advance its cursor.
**Tech stack:** Rust1.99 and Python source extractors, with private source-bound original resources.
**Baseline:** fresh main d5e68a488892e7c81731bf05081bc1c0a9e2687f.

## Scope and evidence

The existing Domination population admits eight APCs with passenger physical seats2–9 and eight motor-pool source vehicles without admitted seats. Existing retail property/archive/registry digests match the retained Domination source inventory for all three factions. Extraction must verify those exact original resources again and keep them private. Driver/gunner authority and arbitrary source modes remain outside this fix. No native game run, deployment, account edit, release or released-pin update is included.

## Tasks

- [x] Add a failing synthetic Domination projector case: eight APC rows with eight expanded passenger seats and no motor-pool seats; reject unknown scope, missing visibility and wrong authored role.
- [x] Permit only the existing Acquisition and qualified Domination scope in the projector and runtime visibility validator. Preserve exact seat inventory/hash checks.
- [x] Add and observe Rust REDs for Domination-shaped catalog/startup and reinforcement→area processing. Implement the startup gate using bound nonzero Structural periodic effects; no enabled effect retains optional compatibility.
- [x] Prove hidden passengers preserve health/mount and the due area cursor advances; reject missing/duplicate/foreign binding before startup.
- [ ] Regenerate and reproduce six private ground catalogs and the lifecycle binding against original retail files. Copy current OIC configs and map ability sidecars, update exact digests, and validate all six profiles with and without Helo. Prove recorded source-connected scans after normal passenger reinforcement.
- [ ] Run affected checks, fmt/strict Clippy and bounded full suites/public boundary. Integrate focused public source PR after CI; checkpoint private evidence/TODO and immediately continue APC driver qualification.
