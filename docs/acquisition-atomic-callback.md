# Atomic Acquisition callback adapter

`acquisition_atomic::AtomicCallbacks` owns an immutable controller catalog and an
explicit effect backend for one world incarnation. There is no default backend,
automatic Engine installation or client request route. An authoritative state
machine supplies callback IDs/entries; client assertions are not state entries.

`prepare` creates an isolated backend stage and performs, in order:

1. Current-state entry and complete native pre-script effects, including required
   FSP/vehicle operations and entry bookkeeping.
2. The controller's ordered script commands against complete current observations.
3. Every trailing call through an explicit handler.
4. Assignment refresh, validation, audit and bounded publication preparation.

Missing effects, observations, handlers or qualification return an error. Current
predicates must match the entry and remain stable during script target requests;
an unexpected current-state change invalidates preparation. A partial damage
query cannot count as a successful command.

The backend stage must own its mutable candidate. It cannot publish, write live
authority, or share mutable authority with the original world. `commit` checks
adapter identity, world, revision, event identity and backend source generations,
then records the exact audit/output before infallible installation. Backend
authority must be serialized through that check/record/install boundary. Stage
drop on failure must have no authoritative effects. These are effect-backend
contracts, not properties that Rust can infer from an arbitrary implementation.

Output contains publication data, never deferred authoritative commands. Replayed
callback IDs return cached audit/output without a second handler, record or
installation. IDs outside the bounded replay window are refused rather than
executed again. IDs are monotonic callback identities, not transport sequences.
A new/reset match needs a new adapter/incarnation. Preparations from another
adapter are rejected even if its supplied world key is identical.

## Native-bound primitives

`acquisition_score_rate` handles the admitted integer-argument domain of
Game_SetScoreBleed: float32 conversion, seconds-to-milliseconds, explicit team0/1,
zero disabling, unsigned deadline comparison/wrap, and preservation of an earlier
pending deadline. Due processing requires deadline strictly before now and
produces at most one deduction per team per update. It returns intents; winner,
score application and match-end rules are separate. Existing bulk elapsed-time
clocks are unchanged.

`acquisition_volume_damage` dispatches one currently resolved penetration pair.
Characters and GroundVehicles use numeric team exclusion `uint8(1-applyToTeam)`;
mines do not. Other classes, including vehicles outside the native GroundVehicle
cast, receive no request. Pair order/multiplicity must be preserved. Actor
ordinary/lethal requests need complete damage/death handlers. The finite-domain
mine helper reproduces accumulation and the detonation-flag mutation; later
qualified mine Update/explosion consequences remain required.

These primitives do not discover overlaps. Authoritative body coverage, current
filter state, native penetration/orientation behavior, liveness resolution,
immediate respawn/vehicle lifecycle and complete assignment/bookkeeping handlers
must be qualified before installing a live backend. The initial-load objective
catalog remains pre-MissionStart only. Synthetic owned effect worlds and private
graph model checks establish adapter orchestration/rollback, not native physics
or live gameplay acceptance. Releases remain manual after live-match review.
