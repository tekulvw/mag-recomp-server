# Acquisition gameplay implementation plan

> Execute autonomously with focused parallel implementation and test-first affected checks.

**Goal:** Complete the authoritative Acquisition loop through results and a fresh round.

**Spec:** ../specs/2026-11-19-acquisition-gameplay-loop-design.md

**Architecture:** Concrete owned objective, ground, character and round candidates use existing source-bound rules and ordinary reliable publication. Native/source data is external; public tests use invented worlds.

## Tasks

- [x] Integrate the existing filtered-static respawn dependency after CI.
- [x] Apply actual volume overlap results to ordinary character and ground lifecycle candidates; test multiplicity, rollback, stale authority and query failure.
- [x] Connect capture/current-target controller, FSP activation/existing Escort respawn, ordered volume effects and assignments to Engine. Test replay, startup, stale rounds, record failure and late join.
- [x] Add explicit driver-seat authority, owner pose decoding/validation and peer replication. Test unauthorized reports, switching, exit/disconnect/death and retained pose.
- [x] Qualify and implement delivery, destruction notification and reinforcement. Test exact source identity, detector rejection, passengers and replacement behavior.
- [x] Connect source-bound score bleed/delivery awards to results, terminal and fresh-match reset/requeue. Test deadline equality, duplicated awards, both outcomes, old requests and two consecutive rounds.
- [ ] Validate all six source-derived faction profiles privately, run affected/full source checks and integrate the focused source PR.

## Pre-flight and decisions

Controller and round share exactly one Rates instance owned by the round candidate. Driver updates and respawn share ground Runtime authority; the Engine serializes installation. Original extraction receipts do not establish live acceptance. Read-only Ghidra analysis is allowed when needed for gameplay predicates. Gameplay-first scope explicitly defers exact native physics, optional mines and cosmetic/statistical parity.

## Source verification

All490 Rust checks (including local subscription sockets),145 Python checks,
strict Clippy/fmt and the886-file public boundary scan passed on the complete
working source. The private twelve-profile registry/six Acquisition Engine
preflights and original-native scoring, driver, explosion and wave receipts
passed separately. Review regressions cover wrong-route grenade reports,
incomplete setup/driver admission, and completed/cancelled hold-exit messages.
The remaining checklist item is the checked source PR integration and exact
private checkpoint; native live-match review remains the maintainer's next
acceptance step, with releases manual.
