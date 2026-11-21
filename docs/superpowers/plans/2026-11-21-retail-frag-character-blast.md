# Retail frag Character blast implementation plan

**Goal:** Make authenticated ordinary retail frags damage current on-foot Characters
in explicitly configured Domination source profiles without a development switch.

**Architecture:** Retain the admitted owner-reported projectile, authored fuse and
outer record-first event transaction. Reuse the source-qualified immutable
sphere/body, current stance anchor, radial falloff and layer-24 material collector
with a typed frag dose. Queue exact reservation/life-bound native damage through
the normal owner reply path, including direction and allocated projectile source.

**Baseline:** Fresh public main `fc95305e44fccaaabe253001bd60f33b83413cdf`.
Both SDK mains were freshly fetched; private unrelated state is preserved.
The human explicitly requests autonomous continuation and checked integration.

## Scope and interfaces

- Extend the existing retail grenade metadata/projector with optional authored
  `material_penetration`. Legacy inputs load; opt-in requires complete finite
  values and enabled retail grenade authority. Do not embed game values.
- Add optional default-false `core.frag_character_blast`, mirrored in World.State
  and Engine, profile selection/retirement, direct and registry validation and
  CLI readiness. It requires existing Character rules, current dynamic bodies,
  static map collision and structural material/sequence inputs, independently
  of hospital rocket equipment. Missing required inputs fail before readiness.
- Query one immutable owner-event view containing accepted current self position
  and stance plus staged Ground motion before damage modifies candidates. Keep
  live snapshot origin checks intact; never claim a staged snapshot is live.
- Only an admitted current-life, accepted UDP owner terminal at its authored fuse
  with a known position can produce frag effects. TCP, stale/replayed/foreign
  claims, expiry and missing positions do not bypass authority. Source opt-in
  takes precedence over the old development path to avoid double damage.
- Preserve existing queue bounds, per-target reservation/life checks, armor/team,
  incapacitation, statistics/rewards and recorder rollback. Carry native location,
  type, exponent-7 direction and actual family16+slot object source.

## Tasks and verification

- [x] Metadata owner: append projection/strict reader access and focused RED/GREEN
  tests; preserve absent legacy-field behavior; stage actual source combat input.
- [x] Runtime owner: immutable owner scene, shared observation collector, typed
  frag resolution and owner planner hook; test no development flag, fourth slot,
  fuse, cover, current accepted self pose/stance, staged vehicle motion, stale
  authority, no duplicates, queue and record refusal.
- [x] Root: config/profile/default readiness wiring, missing-input regressions,
  docs, bounded combined checks and focused public PR integration.
- [x] Private replay owner: stage both six-profile startup scopes and run actual
  Engine normal frag admission/fuse/damage/field8/kill+XP/cleanup/reuse against
  original rules. Independently calculate dose/direction. Freeze and match source
  and input hashes. Scripted positioning and optional incap-off remain explicit.

Native grenade flight/bounce, blocked-ray fallback, exposed mounted targets,
weapon-index presentation and native live-match acceptance remain separate.
No services, accounts, game runs, deployments, releases or released-pin changes.
