# Configured independent live worlds

Registry mode lets one Horizon lobby place humans into separate live DME worlds, each backed by its own Rust worker process. It is opt-in: without `MAG_WORLD_REGISTRY`, the existing single-host `MAG_LOCAL_DME` path remains the default.

## Configuration

Start each Rust worker with its normal externally supplied catalogs, a distinct `--bind` endpoint and a stable `--worker-id`. For example, workers `first` and `second` can listen on `127.0.0.1:31001` and `127.0.0.1:31002`. Each process owns one Engine and creates a fresh incarnation identity at startup. Supply enough resources for each worker separately; the registry does not launch or provision workers.

Set Horizon `MAG_LOCAL_DME=1`, `MAG_WORLD_REGISTRY` to an absolute JSON path, and its MAG application setting to retail application `20231`. A minimal synthetic example is:

```json
{
  "format": "mag_world_registry_v1",
  "application_id": 20231,
  "client_address": "127.0.0.1",
  "client_tcp_port": 10473,
  "workers": [
    {"worker_id": "first", "worker_host": "127.0.0.1", "worker_port": 31001,
     "minimum_players": 2, "maximum_players": 64,
     "minimum_players_per_team": [1, 1], "draining": false},
    {"worker_id": "second", "worker_host": "127.0.0.1", "worker_port": 31002,
     "minimum_players": 2, "maximum_players": 64,
     "minimum_players_per_team": [1, 1], "draining": false}
  ]
}
```

Use the reachable advertised IPv4 address in a real installation. Alternatively set `client_address` to exactly `"$MAG_ADVERTISE_IPV4"`; Horizon then reads the address from its `MAG_ADVERTISE_IPV4` environment variable, with the same validation, so a deployment that keeps that variable current needs no second copy. A drain reload resolves it again and rejects a changed address like any other structural change. The Helm chart's `gameplayWorkers` uses this form (see the chart README, "Configured gameplay workers"). All entries share this client address and TCP endpoint, with distinct registered DME world IDs and reservation tokens. This is one integrated Horizon/DME process with one transport listener and character-store authority. It does not support multiple Horizon replicas or arbitrary external DME servers. The integrated internal listener retains TCP `10073` and UDP `10474`; forwarding must match the advertised endpoint.

The strict configuration accepts 1–16 entries, unique worker IDs and worker endpoints, capacities up to 64, a total minimum of at least one and exactly two nonnegative team minima that fit the maximum. Duplicate and unknown JSON fields, incomplete entries and files larger than 65536 bytes are rejected. Worker profile catalogs must support the requested faction pair, orientation and game mode. Each configured worker must advertise `worker_registry_v1` and acknowledge its scoped identity.

## Placement and ownership

A global human queue prioritizes the longest-waiting faction and randomizes a compatible opponent and orientation. Existing compatible live worlds are filled before an idle worker stages a new match. Team minima and maximum capacity apply per worker. A staged batch is installed as a whole before local formation runs. A worker without bot fill still forms a new match only with at least two humans, one on each side, whatever its configured minima. A worker that advertises `bot_fill_v1` forms its bot-filled profiles under its configured minima as written, so `"minimum_players": 1` with `"minimum_players_per_team": [0, 0]` lets one human start a match that bots fill on both sides. This is opt-in per worker and per profile; additional humans are placed into compatible live worlds first, as above.

To serve several modes at once, run one worker per mode, each with its own `--match-profiles` manifest and `--worker-id`, and list each worker here. For example, three bot-filled workers for Sabotage, Acquisition and Domination each use `"minimum_players": 1, "maximum_players": 64, "minimum_players_per_team": [0, 0]`. Each mode still advertises every ordered faction pair.

Account ownership is global. One account cannot hold live reservations in two worlds. Requeue from a completed match retires its old transport reservation before a new worker admission, while outstanding settlement receipts stay with their original worker and match. Shared durable progression is serialized in Horizon; a committed receipt refreshes the character snapshot of its current owning worker without changing that worker's match XP or replaying rewards.

Each scoped bridge binds one worker incarnation. Requests, subscription frames and receipts must retain that exact identity. Worker restarts are not automatically adopted by an existing host. A lost stream lease or sticky identity fault stops new admission to that entry; healthy workers can continue. This does not provide durable recovery of a crashed worker's lost reward outbox.

## Private parties

Configured retail registries can opt in with `"party_invite_seconds": 30`
at the top level of the JSON. The supported range is 1–300 seconds and
omitting the field leaves party matchmaking disabled. Start at least one
eligible profiled gameplay worker with `--party-reservations`. Its discovery
reply supplies actual per-profile squad capacity and counts for each side.
A worker without those fields can still admit solo players. Standalone
lobbies continue to offer solo matchmaking only; to offer parties from a
single gameplay world, run it as a one-worker registry.

Private invites bind current authenticated accounts and their fixed selected
characters. A party is same-faction and its leader is first in the roster.
A party holds at most eight members, one authored squad. Outstanding
invitations count toward that limit, so an invite that would exceed it is
refused (`reason=party_full` in the Horizon log) and is never delivered.
Only that current leader can queue or cancel the party entry. After
publication, a cancel releases the original entry's still-unclaimed
reservations and leaves claimed members in the match. A leader leaving
dissolves the party. An invite response carries no generation
on the wire, so a party never reissues an invitation to the same account.
Expiry, departure and capacity are replacement policy. This scope does not
emit the native decline-presentation event or support open parties and voice.

A group counts toward the human startup and team limits as one indivisible
entry. It occupies one authored squad in a new formation. Solo late joins
still use the ordinary placement path.

### Late party entry

A worker that also advertises `party_late_entry_v1` (every
`--party-reservations` worker of this version) accepts a party into its
running match. Placement offers a waiting party to running compatible
worlds before anything else: the oldest party goes to the fullest running
world whose mode and faction pair include the party's faction, whose side
and world have human room for every member, and whose squad capacity holds
the whole party. Bots do not count against that room. With no such world,
the party falls back to a new formation as above. A running world takes one
party at a time; a refused or unconfirmed late entry rolls back that party's
cohort, returns it to the global queue and closes the world to parties for
five seconds, without disturbing the running match or its solo admissions.

The worker places the group on its faction's side, in one squad when one
has a free seat for every member (bots in that squad give up their seats),
otherwise across the fewest squads on that side, emptiest first; the reply's
`squad` then names the leader's squad. Bots holding a member's player slot,
overfilling its squads or pushing its side past team capacity retire at
once, and the ordinary bot reconciliation keeps humans plus bots within the
profile's fill total thereafter. Horizon prepares every selected
party before releasing any descriptors for that formation. The worker
acknowledges cohort, formation, profile, world, admission epoch, squad and
ordered reservation IDs. Horizon revalidates all owners before exposing
their DME tokens and then continues with ordinary solo admission. An
uncertain reply is reconciled by the same cohort identity. A prepublication
failure rolls back the attempted cohorts and ordinary per-person transport
lifecycle applies after publication.

## Drain and reconstruction

Replace the configuration file atomically to change `draining` flags. Horizon checks once per second and publishes one complete drain-policy snapshot. Draining blocks new placement and leaves existing world processing active. Other configuration changes are rejected on reload and require controlled reconstruction. A replacement Rust process likewise requires reconstructing its Horizon host rather than rebinding old reservations to a new incarnation. Preserve outstanding receipts and owned sessions when planning that operation.

## Verification

The normal Horizon check covers strict configuration, placement, exact cancellation, transport ownership, drain publication, world dispatch, terminal transfer and shared progression using invented loopback workers. The additional check below starts two actual owned Rust processes with invented catalogs and drives Horizon's registry against them:

```sh
docker build --target check horizon
docker build --target check --file tests/Dockerfile.world-registry .
```

Rust tests separately cover wire fences, subscriptions, restart identity, release-before-reserve ordering and progression-refresh atomicity. These are source and synthetic integration checks; native live-match acceptance remains a separate step. Configuration examples contain no original game data. Source integration does not authorize deployment or release publication.
