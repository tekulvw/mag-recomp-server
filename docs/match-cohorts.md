# Match-cohort results and requeue

The worker advertises `independent_match_cohorts_v1` on `hello`. Horizon uses this
capability to admit a successor while finished players remain in results, release
replies are delayed or durable reward/desired-kit acknowledgements are pending.
Older workers retain the existing admission barriers. This is one live gameplay
world plus finished results delivery, not simultaneous live multi-world hosting.

## Rollover and isolation

A new `select_match_profile` formation archives a terminal cohort and installs a
fresh selected world after one successful journal record. An exact formation
retry preserves its current world; an unfinished match cannot be replaced.
Fixed-profile workers combine rollover and the fresh reservation in a single
`reserved` record. Validation or recording failure installs neither half.
Initial squads/spawns come from reset authored policy, not the old squad's choice.

Each retained cohort owns its actors, immutable wire profile/mode, frozen round
scores and outcome, terminal identity, participation and reliable replay queues.
New actors have fresh match totals, life and replication sequences. No old
movement, combat, repair, leadership or spawn request reaches the new world.
Authenticated time/ping, exact old NAK replay and release remain available.
Archived event replies use the original wire profile and terminal identity;
duplicate reward ACKs do not announce a successor's terminal metadata.

Horizon gives each cohort a separate DME membership and player-slot allocator,
sharing only the registered wire WorldId, clock and existing worker subscription.
Routing checks the exact reservation, client object and owning DME container.
Late disconnect/join notifications and old teardown cannot remove or notify a
successor using the same numeric player slot. Result event traffic uses a
separate bounded mailbox so a delayed old RPC cannot block live gameplay input.

## Ordered results and durable work

On a subscribed worker, only the world stream drains retained terminal notices.
Event/release RPCs do not consume that FIFO. Reward ACK deltas for retained actors
are appended after their queued end notices. The order remains EndCountdown,
EndOfGame, reward statistics/level changes, final character statistics,
CharacterStatsComplete and PostGame, with PostGame waiting for the original end
deadline. Full FIFOs defer whole result groups instead of marking dropped notices
as delivered. Replay bytes and original sequence numbers remain unchanged.

Results wait only for rewards belonging to their own match. Desired-kit receipts
and rewards retain their original exact reservation/world/player identities.
Match XP, FRAGO/statistics and repair carry settle in the originating cohort,
including after its last actor leaves. Durable XP/leadership can update the same
authenticated account and character in a successor; that successor's match
counters remain independent. A reused slot or another character receives no old
credit. Stale snapshots cannot rewind newer durable state. All ACK plans record
before installation, and duplicate acknowledgements remain idempotent.

Horizon serializes durable store writes against admission snapshots, but does not
hold that admission gate while waiting for an old progression ACK. Receipt and
release retry workers cannot stall the ordered stream. Definitive rejected
prospective reservations are removed from the reward-owner ledger; successful or
uncertain admissions retain their owner for late receipts.

## Bounds and remaining acceptance

There are at most eight retained cohorts with actors or unsettled reward work.
A ninth rollover returns `results_cohort_limit` without removing the current
round; admission can retry after old work settles or transports leave/expire.
Transport deadlines remain 30 seconds before claim and 120 seconds of bound
inactivity. Each actor retains the existing bounded queues/history. Combined
bridge replies stay within 128 messages and 24,000 payload bytes; large level
fanout defers packets through the appropriate reliable queue. Fixed-profile
reservation retirement retains a bounded 512-ID history, while registry workers
keep their existing formation/history bounds.

The source tests exercise an active old results client, new faction formation,
slot reuse, delayed release/reward/event replies, record failure, local reward
gating, native result ordering and cleanup. Synthetic fixtures do not establish
native client presentation or multi-human live acceptance. No catalog or game
data changes are required by this increment. Live review remains necessary before
the maintainer manually cuts a release; no source check changes release pins,
services, deployments or the manual release decision.

Configured independent live workers are described in [Configured independent live worlds](configured-world-registry.md). Results cohorts retain their original settlement owner when a character transfers to another worker.
