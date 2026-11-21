# Configured World Registry Implementation Plan

> Root implements registry/configuration/global queue/program integration. Parallel Sol6.2 implementers own worker fencing, scoped bridge fencing and LocalDmeHost parameterization. Standing autonomous fast-loop directions supersede repeated design approvals.

**Goal:** Coordinate two or more explicitly configured independent live Rust/DME worlds through one Horizon lobby and persistence authority.
**Architecture:** Global queue and exact host/world dispatch, one Engine/process per entry, immutable worker-incarnation fencing, per-host liveness/capacity/drain and shared durable store.
**Tech Stack:** Rust1.99, C#/.NET10 and synthetic loopback checks.
**Spec:** ../specs/2026-11-21-configured-world-registry-design.md

## Constraints and review focus

No game/deploy/releases/account stores/private original catalogs. Legacy default stays single-host. Exercise incomplete/duplicate config, queue/session replacement and cancellation, wrong world/worker/slot, lost leases/drain/restarts, deferred result transfers and exactly-once durable settlements. Require two active independent worlds, not two instances of prior results retention.

### Task 1: Incarnation-fenced Rust worker
- [x] Add opt-in --worker-id, fresh worker_identity responses, exact envelope fencing before engine/subscription mutation and read-only discovery.
- [x] Observe missing/stale fence REDs, then correct/control/subscription and restart GREEN; preserve legacy bridge shape.
Files: gameplay source/main and synthetic tests. Interface worker_identity has worker_id/incarnation_id, capability worker_registry_v1.

### Task 2: Scoped bridge and parameterized host
- [x] Bind configured bridge identity exactly once; validate every ordinary reply and subscription frame, fail closed on incarnation change.
- [x] Add immutable LocalDmeHostOptions, registry-owned startup/ticks, placement/account snapshots and terminal transfer, exact staged formation and drain/lease checks.
- [x] Verify two real DME memberships, shared manager ownership, stale transport/reply isolation and per-host lifecycle with synthetic workers.
Files: GameplayBridge/Response/WorkerIdentity, LocalDmeHost*.cs and focused test files.

### Task 3: Configured registry and application wiring
- [x] Write strict config/placement tests first; implement two-to-sixteen explicit entries and drain-only reload.
- [x] Implement global ordered faction queue, exact ticket cancellation/callbacks, account ownership and batch placement into acknowledged per-world profiles.
- [x] Aggregate status, dispatch DME input to exact owning host and centralize only global transport startup/ticks/shutdown.
- [x] Drive two simultaneous independent worlds, one failed/drained/full entry, terminal transfer and shared progression through synthetic loopback tests.
Files: new MagWorldRegistry*.cs, Program.cs/MLS.cs and Tests.MagPlugin.

### Task 4: Verification and integration
- [x] Run bounded affected checks, full Rust/Horizon/Python checks, fmt/strict Clippy/build and public boundary.
- [x] One fresh combined review; fix concrete findings with RED/GREEN regressions.
- [ ] Focused public source PR, green CI and exact merge-tree verification; private TODO/evidence checkpoint retaining released pin and manual release PR.

## Combined review resolutions

The fresh combined review found five concrete lifecycle issues. Fenced read-only hello polls now preserve pending receipt batches in their response; an unknown release records a tombstone before acknowledgement so a delayed reserve cannot resurrect it; cancellation targets the exact session/request even after the success callback; drain reload publishes one immutable policy snapshot; and listener startup/shutdown retain exact transport ownership through partial failures. Each behavior has an observed failing regression and a corrected check. The additional real-worker Docker check exercises two independent Rust processes through the Horizon registry and is included in the Horizon CI job.
