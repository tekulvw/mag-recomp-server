# Acquisition World Queries Implementation Plan

Execute inline with independent native/lifecycle investigations. Standing user
instructions authorize autonomous implementation and checked source integration.

Goal: concrete world contact/sweep queries and actual lifecycle consumers for the
first playable Acquisition loop. Rust2024; no new dependencies. Spec:
../specs/2026-11-02-acquisition-world-queries-design.md

- [x] Add failing static/dynamic contact, sweep, filtering, exact-root exclusion,
  starting escape, work limits and incomplete-world tests.
- [x] Extend convex distance bounds/normals, bounded penetration and translational sweeps.
- [x] Load explicit static filter/radius provenance and expose bounded BVH
  triangle candidates; preserve existing ray/AABB APIs and resource compatibility.
- [x] Implement the actual spawn-blocker world provider over static and dynamic
  geometry, including complete filters and owned moved-character pose.
- [x] Add mutable ground current authority and qualified lifecycle consumers;
  collect collision bodies from current authority instead of initial resources.
- [x] Add ordinary immediate character effect consumers and metadata-correct
  notifications/rewards without owner-update deferral.
- [x] Add actual owner field5 corrections with exact correction ACK locking,
  final recipient sequence binding and staged rollback.
- [x] Integrate existing-vehicle respawn, per-pair push/damage/filter effects and
  source counters in one recorded Engine transaction; register roots on Update.
- [x] Qualify initial compiled/terrain root rejection for all three maps;
  filtered respawn queries need no geometry/child data for rejected roots.
- [ ] Complete objective integration/driving/destruction/scoring consumers for
  the first playable mode. Other enabled world queries still need their data.
- [x] Run affected/full checks, private source/resource consumers and boundary
  verification. Reconcile main, integrate green source PR, checkpoint private
  evidence/TODO; preserve released pin/data and manual releases.

Review focus: static metadata absence is an error; mixed collision roots retain
identity; current filters include group exceptions; separating floor contact
does not prevent escape; repeated victims/respawns retain correct generations;
missing lifecycle consumers must never be represented as completed effects.
