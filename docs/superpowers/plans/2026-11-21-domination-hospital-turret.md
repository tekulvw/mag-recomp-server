# Indexed Domination hospital primary turret implementation plan

> Continue autonomous gameplay-first TODO work with Sol6.2 implementation agents. Qualification is source-bound to original retail files; no per-feature approval prompt, deployment or release is included.

**Goal:** Enable an explicit indexed hospital primary hitscan path across all six existing Domination profiles, with qualified seating, ownership, equipment and damage.
**Architecture:** Optional external Core.hospital_turrets rules bind exact hospital profiles/models and the attested retail combat rows. Match-owned per-index state retains aim and a fresh equipment grant; pure retail codecs remain separate from legacy Alpha. Damage reuses the existing character armor/health/kill transaction.
**Baseline:** freshly fetched main3a4de1f0c15f9844b35f5a6b3caf26c275249846; SDK ancestry refreshed, native dirty state preserved.

## Proved retail facts

Wire qualification verifies60 spans,95 words,61 branches and3 original property variants. Family13 has16 allocations. Turret0 owner field6 governs yaw8(sign1/int3/frac7=11 bits),pitch9(sign1/int1/frac8=10),primary11,secondary12 and overheat21(raw8). Server-only inventory10 has version8 plus3 presence-gated12-bit IDs. Original populated definition version2 has [Weapon1,Weapon2,absent]. Faction primaries1107/2127/3152 are mounted miniguns; secondary definitions1108/2128/3153 remain replicated but do not authorize secondary damage. Primary gear0,pool0 and fire-mode index0 are distinct checks. Hit020f omits attacker/grant; native uses the admitted occupied Character and source_object0xffffffff.

Pose qualification verifies each authored name's explicit gunner_seat fallback to model root0, identity local transform, hidden seat0/turret0. It is not the passenger empty-name fallback. Original hulk activation/refresh uses body heading plus the same last turret0 relative yaw about world Y. No muzzle or rebuild-yaw-reset behavior is inferred.

## Grant policy and boundaries

Reserve original populated version2 for initial definitions, then issue non-reused versions3..255 per hospital match lifetime as a conservative replacement fence. Reject an exhausted new grant without mutating occupancy; duplicates reuse the committed grant. Preserve the counter through destruction/rebuild. Fresh round/transport scope can reset it. Preserve authored inventory slots because native local weapon creation is separate from field10. Until an exact-version primary report establishes readiness and an owner-sequence boundary, deny aim-only mutation and firing. Native same-connection owner reports share one increasing sequence; arbitrary forged/unseen Hit replay cannot be classified by this wire and is not claimed.

Keep last accepted aim across seat release and use it for source-qualified hulk heading. Every shot still requires current operational hospital, exact exclusive seat0/reservation/living generation, initialized primary gear0/pool0 and requested gear0/mode0. Mounted-only combat rows must never enter on-foot or unrelated vehicle damage constructors. Preserve native callback/effect and record-before-install semantics, recipient/packet bounds and legacy no-opt-in behavior. Ground gunners/secondary rockets and unrelated hidden-hospital area fidelity remain separate.

Final transaction review reproduced two connected publication defects: a grenade launched before mounting could consume the queue slot preflighted for aim, and a ready loading peer could receive family13 before creation. Charge all same-event publications before recording aim, dropping only the aim candidate when capacity is unavailable. Preserve queued Character/seat transitions and gate their drain until hospital creation. A separately bounded one-time bootstrap reservation is necessary so an already-full ordinary queue cannot deadlock creation; ordinary reliable capacity remains128 and no queued packet is discarded or coalesced.

## Work split and interfaces

- Runtime agent owns hospital_turrets.rs (strict rules/state/grants), structures/seat/event ownership integration, optional Core configuration and late join. It exposes Engine::hospital_primary_for_actor(active,hit) -> qualified item/equipment/version after authority checks, and has_hospital_turrets().
- Wire agent owns hospital_turret_wire.rs and codec tests: definition version/items, lossless yaw/pitch types, sparse client rows for8/9/11/12/21, server definition/update batches with strict index/order/field caps.
- Combat agent owns mounted-only RetailBulletUsage and constructors, CharacterHitProfile integration and accepted-route/round checks. Coordinate lib.rs module declarations rather than overwrite other edits.
- Root owns maintained original-file rule/combat projection, synthetic extractor tests, documentation, combined verification/source integration and private evidence. Projected rules bind native image/archive/proof/model/structure/attested-item identities and all six exact hospital placements.

## Tasks

- [x] Qualify original retail wire/equipment/hit semantics and named seat/pivot/hulk path. REA adapter explicitly rejects PPC ELF; existing direct LLVM/original PT_LOAD checks supply the recorded evidence. No installation or target execution.
- [x] Add/observe focused REDs, implement pure retail codecs, and verify exact widths/field ordering, three slots, malformed masks/indexes/duplicates and packet bounds without touching Alpha behavior.
- [x] Add/observe grant/lifecycle/config REDs; implement exact optional role0, fresh version/readiness, indexed mutation/relay only after record, two-owner isolation, duplicate mounts, remount/death/destroy/rebuild/release and bounded late join. Final combined-publication and pre-creation recipient regressions are part of the remaining verification task.
- [x] Add mounted-only combat rows and validated primary constructor; prove source-bound damage/armor/stat/kill behavior through the existing transaction, and reject bare/on-foot/foreign/secondary use and failed-record leaks.
- [x] Build maintained extraction from owned original files/attested catalogs, project exact variant/profile/pose/weapon bindings, regenerate combat and stage six current OIC profiles with collision/ability digests preserved.
- [ ] Run focused then bounded full checks,fmt/strictClippy/build/public boundary and actual six-profile source-only connected primary/ownership/hulk/latejoin replay. Integrate focused public PR after green CI and exact merged-tree proof; checkpoint private TODO/evidence with released pin unchanged and continue.
