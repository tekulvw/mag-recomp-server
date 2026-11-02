# Acquisition APC Insertion Implementation Plan

> **For agentic workers:** Execute native binding/exit analysis and the isolated
> Rust insertion work in parallel; root integrates the projection and checks.

**Goal:** Supply source-qualified Acquisition spawn/APC inputs for every faction
and implement safe static APC passenger entry/exit.

**Architecture:** Explicit external binding/property/exit inputs feed the existing
spawn and ground catalogs. Keep separate vehicle and player spawn identities,
typed passenger state and the existing journal/replication transaction path.

**Tech Stack:** Rust, Python stdlib, original read-only PPC/resource analysis.

**Spec:** `docs/superpowers/specs/2026-11-02-acquisition-apc-insertion-design.md`

## Global Constraints

- Public source and invented fixtures only; original data remains private.
- Preserve faction selection, accounts, native indices, services and release pin.
- Do not invent pool order, grounded positions, seat defaults or native parity.
- Releases remain manual after human live-match review.

## Review Focus

- DEV and retail family/field differences must not silently transfer.
- Every spawn link/default must follow the same verified index remapping.
- Vehicle creation precedes mounted references for initial and late joins.
- Occupancy, cleanup and replay remain per vehicle and transaction-safe.
- Exits/remounts need explicit qualified policies rather than Alpha assumptions.

### Task1: Original Acquisition binding

- [x] Reproduce216 compiled transforms/topology from original3maps.
- [x] Close exact factory pass/family/allocation and source pool identity.
- [x] Bind original APC properties/model bounds/exit contract and qualify poses.

### Task2: Rust insertion

- [x] Write critical synthetic red tests for all3factions and independent seats.
- [x] Implement external bindings, typed occupancy, spawn/snapshot/cleanup/replay.
- [x] Require qualified exits; protect manual remount range/freshness.
- [x] Preserve Alpha and common mounted guards; focused tests green.

### Task3: Maintained projection

- [x] Add invented remap/property/exit tests and observe red.
- [x] Project bound pools and faction APC catalogs without compiled asset values.
- [x] Reproduce all six Acquisition faction pair profiles; validate real consumers.

### Task4: Integrate

- [x] Run affected full Rust/Python checks, Clippy/fmt/public boundary locally.
- [ ] Confirm public CI before merging.
- [ ] Merge checked public source and checkpoint private tools/data/TODO.
- [ ] Keep native acceptance, moving vehicles and manual release boundaries clear.
