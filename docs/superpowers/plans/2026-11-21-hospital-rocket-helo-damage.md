# Hospital rocket Helo damage

**Goal:** Extend the optional equipped hospital rocket transaction to qualified
current operational Helo targets using the original vehicle callback.

**Baseline:** Fresh main `f6e1d97b9e72bdae7d75c50aaec3024c93620995`.
Both SDK mains refreshed; manual releases and unrelated state are preserved.
Original qualification is retained privately under hospital-secondary-rockets/helo-blast.

## Scope and interfaces

Original family2 dispatch uses the vehicle dose, outer body overlap, live model
origin, one layer24 material ray, classified areas1–6, type10/zero armor reductions,
existing friendly armor and native flight floors. The callback's vehicle source
object value and the inherited outgoing source word remain unqualified. Preserve
the replacement sentinel and record allocated projectile identity in audit.

Optional `core.helo.hospital_rocket_damage_areas` requires hospital rockets. Store
its validated three-faction catalog only in World.State. Bind original ELF/Main,
loaded Helo models/damage document hashes and exact operational packs. Preserve
no-Helo and absent-field behavior. Share the existing pure Ground area classifier
without changing its f32 arithmetic or classification boundaries.

An immutable Helo planner checks current fleet/mesh pose correspondence, row6
layer22 eligibility, actual chassis overlap and target-excluding material cover.
Apply classified vehicle armor to a cloned fleet through existing HullDamage.
The normal provider retains critical/death/passenger/retreat/recovery lifecycle.
Dormant targets are absent; missing or mismatched active geometry is an error.

RocketEffects adds optional Helo candidate, audit and packet count to the same
Character/Ground/terminal preflight and record. Install all candidates only after
record succeeds. Family2 damage is queued only after Helo creation for a recipient;
late join creation embeds current field18. Queue pressure defers all effects.

## Tasks

- Bounds owner: maintained original-input projector, strict three-row area reader,
  pure shared classifier and source-binding/rotated-boundary regressions.
- Runtime owner: typed Helo-only candidate, live geometry/cover/armor/damage/field18
  tests and combined transaction regressions; no generic area0 substitution.
- Root: optional path/scope/source validation, State/CLI readiness, RocketEffects
  staging/installation, docs, final focused checks and source PR integration.
- Replay owner: normal original AA/FSP activation and boarding, then explicitly
  scripted coherent Helo pose/mesh near admitted rocket terminal for all variants;
  verify independent area/dose/field18, combined terminal, creation-before-damage,
  latejoin/cleanup/reuse and final source/input hashes.

Native flight/input and exact Havok/frame ordering remain separate. Friendly
relation and nonzero flight floors use synthetic checks because original qualified
Helo profiles have their own fixed team/floor configuration. No game, services,
accounts, deployment, release or released-pin change is included.
