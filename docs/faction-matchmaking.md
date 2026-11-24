# Faction-aware human match formation

An explicit worker `--match-profiles` registry enables faction matchmaking in
Horizon. Fixed-profile operation remains available when that capability is absent.
See [worker profile configuration](worker-match-profiles.md) for external inputs.
Horizon requires all six ordered faction pairs for every mode advertised by the
registry. Profiles select the defender-owned home map; saved characters retain
their own faction, equipment and account identity.

For Sabotage and Acquisition:

1. Queue authenticated human intent before allocating a DME client or reservation.
2. Wait for the configured total minimum and at least one human on each selected
   side. Larger configured per-side minima remain effective. Bots do not count.
   Exception: a profile the worker fills with bots (`bot_fill_v1`, see
   [bot-filled profiles](worker-match-profiles.md#bot-filled-profiles)) uses the
   operator's configured minima without that floor. With a configured minimum
   of one and per-side minima `[0, 0]`, one human starts the match in their own
   faction, as attacker or defender, and bots fill both sides.
3. Prioritize the longest-waiting eligible faction, randomly choose an eligible
   opponent, then independently draw a feasible attacker/defender orientation.
   Map/profile counts do not weight faction selection. Other factions stay queued.
4. Acknowledge the exact profile and formation identity on an idle Rust worker,
   then configure its human quorum. Only then admit the selected cohort.
5. Lock the pairing for that cohort. Late joins fill only its two factions within
   team/world capacity. Once the round is terminal, workers advertising
   `independent_match_cohorts_v1` can retain its results separately and admit a
   next cohort with a different pairing and map. Old releases and reward ACKs
   do not block unrelated next-cohort admission. Legacy workers keep their
   existing all-members/all-receipts barrier.

Waiting does not block MLS keepalive, status or cancellation processing. Refreshes
preserve queue age and replace callback ownership; duplicate accounts count once.
Disconnected, expired, replaced or cancelled sessions cannot inflate population.
The pending queue is bounded at4096 human owners.

Retail waiting updates use the original0014 layout: common header, one-bit
success, signed eight-bit raw queue enum, then12 unaligned u32 counts. Updates
every five seconds refresh the original30-second pending timeout. The installed
MAG user callback ignores the payload. Its raw enum is therefore serialized as
local compatibility value0; this is not a recovered game-type mapping. Retail
cancel0015 carries a u32 party field, but cancellation authority comes from the
bound MLS session. No cancellation acknowledgement/result code is invented.

An uncertain profile acknowledgement is retried with the same formation identity.
A failed/mismatched acknowledgement never admits players to the old map. The
worker refuses switching unfinished gameplay worlds or dropping unsettled
reward/kit work. Finished result transports have separate membership and player
slot allocators under the same wire world ID; see [cohort lifecycle](match-cohorts.md).
Queue callbacks and cleanup retain their owner/session checks.

The source supports one active DME/Rust world, with queued waiting cohorts.
This is not concurrent multi-world hosting. Match capacity remains at most64;
authored128-player Acquisition is a separate milestone. Global legacy
`MagAllowedFactions`/`MagGameType` availability applies to fixed operation;
in registry mode the acknowledged profile matrix controls served modes/factions.
Use a retail0/1/3 base configuration when selecting a64-player Horizon maximum.

Profiles must supply all needed resource bindings. The current all-map projection
is entry groundwork and leaves unproved spawn identities and unsupported vehicle
types unavailable. Arulkum APC-backed insertion remains required. Registry mode
refuses the old global bot-population, bunker, leadership and dummy extras rather
than carrying stale map-specific state into another profile. Bots are instead
configured per profile and validated against that profile's own world.
Native queued join/cancel, map loading and gameplay acceptance remain live checks.
Passing source tests does not change services, release pins or the maintainer's
manual release decision after live-match review.

For a shared human queue across explicitly configured independent live worlds, see [Configured independent live worlds](configured-world-registry.md). The single-host path remains the default.
