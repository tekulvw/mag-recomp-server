# Acquisition ordinary frag Ground damage correction

**Goal:** Correct ordinary frag damage to Acquisition APC, motor-pool and Escort
vehicles using the proved debris vehicle channel and current material scene.

**Baseline:** Fresh main `89a0b7af284d3815b48114eea725238de8f069ac`.
Both SDK mains refreshed. Manual releases, unrelated state and old evidence stay.

## Proven problem and bounded policy

The current generic path uses authored prevention reduction, body area0, a
three-ray opaque average and committed collision poses. The original callback
uses vehicle dose, zero reductions, classified model areas1–6 and one material
ray to the current origin. The discrepancy materially increases vehicle damage.
All9 role/faction chassis bounds and all6 current source material worlds are known.

The original wheel box carries a resolvable vehicle identity and participates in
layer6 overlap. Include either bound chassis or wheel positive overlap. Apply one
dose per vehicle key per terminal as explicit replacement policy; native repeat
callback parity remains deferred. Exclude the whole selected key from its own ray
and retain current other-vehicle wheel cover behavior. Do not reuse the rocket
flight solid-contact wheel exclusion for frag blast admission.

## Architecture and ownership

Optional Acquisition `frag_ground_areas` loads a strict9-row source catalog into
Setup. It works independently of hospital rockets and the Character boolean,
requires existing native Structural/material/current-body/Ground policy inputs,
and validates explicit 3/7/9 source roles,16 current placements, exact dynamic
catalog/template/root topology and original bounds. Absent option preserves legacy.

- Bounds owner: maintained original-input projector + strict external catalog,
  all9 original selected chassis (Bushmaster pack1/tag9, not turret), both bound
  root roles, dynamic source identity and synthetic reader/projection tests.
- Runtime owner: config/Setup loading and public validation methods, common
  source-frag fresh-owner predicate, typed vehicle-only planner and owner hook.
  Use the same staged Ground runtime for snapshot, model anchor, classifier and
  damage. Clone/apply per terminal, skip dying targets and emit one change batch.
  Native zeroRedux/area/direction and explicit invalid wire-source policy remain
  separate from Character and legacy vehicle-destruction/Helo callers.
- Root: registry/direct CLI readiness and validation glue, docs, final checks and
  focused public source integration. Avoid a broad unrelated blast refactor.
- Replay owner: normal source Capture enabling Escort, authenticated frag launch
  and fuse, all three Ground roles/current rotated poses and independent armor/FF/
  area/field18 math across six profiles. Verify source identity, rollback, once
  publication/reuse and final source/input hashes; preserve all old artifacts.

The existing outer owner event records before installing runtime/flight/queues.
Typed query failures keep legitimate terminal/motion/Character handling, report the
denial and never fall through to the known wrong generic resolver.

Critical regressions: actual Escort HP correction, staged driver pose, both-root
and wheel-only eligibility with one dose, native armor/FF/area/direction/sentinel,
cover, missing source/dynamic dependency and recorder refusal. Do not claim native
physics, repeat callback order, client acceptance or performance from source tests.
