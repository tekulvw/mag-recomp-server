# Retail 2.12 bootstrap catalogs

**Goal:** Produce source-bound, private character inputs for the first native
create/login test, with an explicit authentication-only startup profile.
**Spec:** `docs/superpowers/specs/2026-11-28-retail-v212-protocol-design.md`.
**Execution:** Inline, focused test-first increments; passing source PRs target
`codex/retail-v2.12`. No release or activation follows from source checks alone.

## Boundaries and selected policy

- New explicit `retail-v2.12-bootstrap` profile; existing legacy extractors,
  source hashes, 667-position attestation and startup defaults stay unchanged.
- Exact supported engine and three effective ZRB identities are pinned. A bounded
  reflective-property reader consumes caller-supplied files, never repository
  assets. Public checks use synthetic byte fixtures only.
- Actual 2.12 item banks, nine authored starter kits and progression values are
  projected. Native22slot order, null handles, bank encoding and ownership-bit
  conversion have source-bound evidence. Loaded-vector order/canonicalization
  and native initial entitlement defaults remain explicit qualification limits.
- Bootstrap operator policy: level1, no additional skill points, three authored
  kits followed by two cleared wire slots, initial update ID1, no purchased-skill
  grants. These policy values are not claimed as original native creation defaults.
- New skill eligibility and training/economy rewards remain unsupported. Do not
  fabricate a legacy Tiers table or reuse a v1 skill/training catalog.
- Profile manifest binds exact source identities and SHA256 of the two character
  catalogs. Runtime rejects missing/wrong profile, stale data, gameplay startup,
  registry wiring, old skill/training inputs and incompatible level overrides.
- Authentication-only main: `MAG_LOCAL_DME=0`, no `MAG_WORLD_REGISTRY`; gameplay
  workloads remain0. Portal/admin/status and isolated existing stores persist.
- Three private outputs: `retail-v212-bootstrap-profile.json`,
  `retail-v212-bootstrap-armory.json`, `retail-v212-bootstrap-progression.json`.

## Tasks and interfaces

1. Create `tools/extraction/reflective_zrb.py`: `Reader(data, limits=...).object`
   returns typed values with source offsets/hashes; unknown types remain opaque.
   Synthetic tests cover truncation, lengths, nesting/node budgets, duplicate
   fields, invalid finite scalars and supported vector/object boundaries.
2. Create `tools/extraction/extract-retail-v212-bootstrap.py`: strict CLI with
   engine/headers/classes/equipment paths and private output directory. Generic
   projection helpers can be tested with synthetic identity specs; CLI accepts
   only the maintained exact2.12 profile. Missing/foreign inputs must leave no
   partial outputs. Existing v1 entry points are untouched.
3. Add `MagRetail212BootstrapProfile` and focused synthetic fixture checks.
   Consume `MAG_CHARACTER_BOOTSTRAP_PROFILE`, `MAG_RETAIL_ARMORY_CATALOG` and
   `MAG_PROGRESSION_CATALOG`; reject other profile/world/legacy catalog settings.
   Bind content hashes before loading existing Armory/progression consumers.
4. Route only the explicit1793 bootstrap selection through the new loader in
   `Program.LoadCharacterCatalog`, with skills/training disabled. Run focused
   checks and fullLinuxCI through a sourcePR into integration.
5. Privately reproduce all three outputs against actual known source files;
   validate with maintained .NET consumers and retain exact receipts/limits.
   Hand bundle/source pins to homelab only when client and data readiness agree.
