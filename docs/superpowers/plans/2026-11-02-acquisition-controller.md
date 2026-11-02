# Acquisition Controller Implementation Plan

> Execute the source increment inline with test-first checks. The user authorized
> this concrete increment and integration; do not add proposal/review gates.

**Goal:** Load external four-chain configuration and plan proven ordered actions.
**Architecture:** A strict bounded Rust catalog/planner consumes external groups;
a nonexecuting Python parser/projector produces them from external source inputs.
**Tech Stack:** Rust, serde, Python stdlib, original read-only native analysis.
**Spec:** `docs/superpowers/specs/2026-11-02-acquisition-controller-design.md`.

## Constraints and review focus

- Preserve record-before-commit, services, accounts, released pin and manual releases.
- Keep original data/code out of public files; invented fixtures only.
- Preserve whole-snapshot branch priority, group/action order and source anomalies.
- Deny missing states, foreign targets, wrong maps/hashes and unbounded inputs.
- Preserve unbound calls as unresolved; do not imply authoritative native events.

## Task1: Rust catalog and pure planner

- [x] Add failing tests for priority/order, complete snapshots, target/source joins.
- [x] Implement `acquisition_control::Catalog::{load,from_json,plan}` and typed commands.
- [x] Add an offline consumer example; prove explicit unbound calls remain visible.

## Task2: Maintained projection

- [x] Add failing invented script/projection tests, including comments/unsupported grammar.
- [x] Parse ordered groups without Lua execution and join typed map parameters.
- [x] Reproduce six original-derived graph catalogs and load/plan them in Rust.

## Task3: Integration

- [ ] Full affected Rust/Python suites, strict Clippy/fmt and staged public scan.
- [ ] Focused public PR, passing CI, merge exact head; private main evidence/TODO.
- [ ] Report requested parallel Rust readability suggestions without automatic refactors.
