# Faction Match Formation Implementation Plan

> **For agentic workers:** Execute the independent projection and worker-profile
> tasks in parallel; root implements the Horizon queue and integrates after checks.

**Goal:** Form faction-correct retail Sabotage/Acquisition matches from real
human queues, selecting sides before map admission.

**Architecture:** Explicit external profile registry, idle worker selection,
nonblocking Horizon queue, longest-waiting faction and injected random draws.

**Tech Stack:** Rust, C#, Python stdlib.

**Spec:** `docs/superpowers/specs/2026-11-01-faction-match-formation-design.md`

## Global Constraints

- Preserve saved factions, accounts, indices, manual releases and live services.
- Maximum64 remains explicit; bots never satisfy the human minimum.
- Profiles/selection must agree before reservation or GameInfo.
- Public source/invented tests only; original resources remain private.

## Review Focus

- Oldest eligible faction must not be starved by repeated new arrivals.
- Distinct factions, roles and defender-owned map stay fixed for a cohort.
- Duplicate/cancelled/disconnected sessions cannot inflate population.
- Configuration failure cannot admit a cohort to an old world/profile.
- Late join/requeue must preserve results and independent queued factions.

### Task1: Every faction's map/profile projection

- [x] Add critical synthetic all-pair/mode tests, observe red.
- [x] Extend maintained projector; preserve legacy output bytes.
- [x] Reproduce all12 original private profiles and run Python/public checks.

### Task2: Worker profile registry and selection

- [x] Add source-only manifest/profile selection tests, observe red.
- [x] Load/validate explicit per-profile resources; advertise typed summaries.
- [x] Add acknowledged idle-only/idempotent selection and preselection reserve gate.
- [x] Test all roles, conflicts, cleanup and startup paths; run Rust/Clippy.

### Task3: Human queue and randomized formation

- [x] Add deterministic synthetic tests for fairness, eligibility, roles,
  minimum/capacity, duplicate/cancel/expiry and fixed late join.
- [x] Implement nonblocking queue, exact worker-selection acknowledgement,
  cohort admission, honest status and failure/cleanup handling.
- [x] Integrate source-bound native waiting/cancel codec if proved; preserve
  unsupported semantics rather than fabricate a success packet.
- [x] Run Horizon checks and affected full checks:277 Rust,112 Python,
  23 Horizon groups; strict Clippy/fmt. Local Docker build cache is full, so
  .NET checks ran in a read-only SDK container with restore/build output on the
  host; shared Docker images/cache/services were preserved.

### Task4: Integrate and preserve private reproduction

- [ ] Reconcile main, public boundary and CI; merge focused source after checks.
- [ ] Checkpoint private profiles/evidence/TODO without changing release pin.
