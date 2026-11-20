# Leadership AutoRepair Implementation Plan

> **For agentic workers:** Use independent file ownership and one focused integration review. Standing autonomous/fast-loop user directions supersede routine proposal approval and per-task reviewer gates.

**Goal:** Enable source-qualified occupied Acquisition Escort AutoRepair.
**Architecture:** Add metadata-preserving percent healing to the existing Ground runtime; stage per-life automatic timers, effective leadership and vehicle/field18 changes in a recorded scheduler transaction.
**Tech Stack:** Rust, existing synthetic Engine fixtures, private retail proof.
**Spec:** docs/superpowers/specs/2026-11-20-leadership-auto-repair-design.md

## Global Constraints
- Fresh public main442b97d/tree93dcb9f and private main1862cdc; preserve unrelated native indices and captures.
- Existing Acquisition/source-type9/live ready occupant scope; source-bound effective modifier and authoritative seat.
- Float delta/HP, strict >1.5-second timer, one pulse/update, remainder, near-zero reset and no fabricated hit cooldown.
- Record before timer/HP/leadership/network installation; peer and late-join field18 retain damage metadata.
- No original assets/evidence publicly; no XERJ, game, deployment, release or released-gitlink update.

## Review Focus
- Idle/duplicate scheduler calls must not multiply heals or give a new life retroactive elapsed time.
- Role expiry, seat vacancy, invalid authority and changed vehicle body must not keep repairing stale targets.
- Full/critical/nonoperating vehicles consume a due interval without unintended health/lifecycle changes.
- Unchanged health projection may still represent a float HP change; preserve accumulated float precision.
- Failed recording must leave the retry's timer/leadership/HP/queues unchanged.

### Task1: Ground health operation (Sol worker)
**Files:** acquisition_ground_runtime.rs and focused new child tests only.
**Interface:** Runtime::heal_percent(key:Key, percent:f32)->Result<Vec<LifecycleEvent>, &'static str> (crate-private). Nonfinite rejects; nonpositive/no-op and unsupported normal life return empty. Existing generation guard, ordinary state2 Operational noncritical only. Return existing DamageChanged on positive float change, preserve metadata, resolve health conditions.
- [x] Add failing real runtime tests for percentage/cap/fraction, metadata/conditions, noncritical scope and stale key.
- [x] Implement the minimal health mutation and verify affected ground tests.

### Task2: Scheduler authority and recorded publication (root)
**Files:** new leadership_auto_repair.rs/tests; lib.rs/resources.rs/world_tick.rs, leadership_runtime.rs and reset hooks as needed.
- [x] Write connected failing Engine tests with actual effective leadership/source Escort/seat resources.
- [x] Stage per-life clocks, current seat and loaded modifier; apply Task1 operation on cloned Ground runtime, encode field18 before recording, then install and fanout.
- [x] Verify timing, remainder, lease expiry, seat/life/round changes, two occupants, deduplication, metadata/current and late health, and record retry.

### Task3: Qualification, review and integration (root)
- [x] Reproduce private source proof, document enabled scope and remaining unknowns.
- [x] Run affected checks and final suites/fmt/strict Clippy/build/public boundary; one focused Sol6.2 integration review and concrete fixes.
- [ ] Integrate focused PR after required CI; verify exact tree and private checkpoint while retaining released pin.

A reproduced range bug requires leadership_runtime to resolve a bound Ground occupant at the current vehicle root, for provider and recipient proximity. Retain the original on-foot path and existing hospital/Helo scope.

Parallel read-only next-item work may qualify living AutoHeal; no automatic expansion of this increment.
