# Domination ordinary frag Helo damage implementation plan

**Goal:** Correct ordinary frag Helo damage and preserve creation-before-field18 publication for the opted-in path without coupling it to hospital rockets.

**Architecture:** Optional `core.helo.frag_damage_areas` loads the existing strict three-faction geometry catalog independently of the rocket option. The original retail vehicle callback and current Helo meshes qualify this area source. Factor a source-neutral Helo vehicle-blast planner from the rocket implementation; keep its rocket adapter and checks. The frag adapter uses the accepted owner/Ground motion scene before Ground damage and all terminal doses in order against one fleet candidate.

**Scope:** Source gameplay only; preserve absent-option legacy behavior, all previous rocket behavior, source evidence, unrelated state and manual releases. Existing legacy Helo already has one cover ray. Correct vehicle zeroRedux, areas1–6, authored material penetration, current basis/root, native direction, and explicit MAX source replacement. One dose/carrier/terminal and callback/native-input limits remain explicit.

## Runtime owner

- Add `State.helo_frag_damage_areas`, Core Helo path resolution, original ELF/Main/models/damage source validation and independent frag metadata/material startup validation. Expose World has/validate helpers.
- Introduce internal Helo credit/dose/frame structs and a shared planner accepting an ordered dose slice, current optional owner collision view, geometry rules and collision frame. Clone one fleet, share budgets, preserve eligibility/floor, apply each dose, publish each carrier's final field18 once and retain per-dose audit.
- Keep the hospital rocket adapter's launch-source validation and singleton committed-scene semantics. Frag adapter captures current authenticated attacker/team/world and admitted projectile IDs, never falls through to legacy after typed failure.
- Plan typed Helo before Ground health mutation so Character, Ground and Helo see the same accepted motion scene. Include Helo-only opt-in in owner freshness/route guards. Set `EventPlan.helo_damage_requires_creation` only for typed effects; legacy path remains after Ground.
- Observe meaningful damage regression RED before implementation; verify both relations, area/direction/MAX, multiple terminals, accepted motion/cover, floor, missing scene denial, rollback and legacy/rocket preservation.

## Root integration/publication

- Extend Registry validation and readiness aggregation; add `frag_helo_areas_validated` CLI metadata.
- For typed Helo field18, require matching world, initialized, ready and `helo_creation_sent` recipients at actual commit. Reflect that recipient filter in turret-aim queue accounting. Retain legacy publication when option absent.
- Preserve ordinary frag's existing bounded `queue_shared` overflow policy: mark the full recipient backlogged and retire it through recorded lifecycle cleanup. Healthy publishers/recipients continue. Do not claim server retry/defer semantics or general reliable preflight.
- Test creation gating/late creation and exact queue boundary with bounded memory and normal retirement; document optional config and precise limits.

## Private replay and integration

- Reuse normal six-profile AA/FSP/boarding from the prior Helo proof, then a reported frag at a coherent scripted current Helo mesh/pose. Ground and Character candidates may accompany it; include one Helo-only source setup to prove independence of rocket/contact and other frag options.
- Check native vehicle arithmetic, all factions, hostile and friendly Helo cases, field18, creation-before, latejoin current HP, terminal/inactive/reuse and recorder refusal. Keep scripted pose, incapacitation-off and native callback/flight limits explicit.
- Freeze and hash source/inputs; run focused checks then final fmt/strictClippy/build/Rust/Python/public-boundary checks. Integrate exact checked source head after required CI, checkpoint privately and continue to the qualified Hospital blast receiver. Releases remain manual.
