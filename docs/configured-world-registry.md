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

Use the reachable advertised IPv4 address in a real installation. All entries share this client address and TCP endpoint, with distinct registered DME world IDs and reservation tokens. This is one integrated Horizon/DME process with one transport listener and character-store authority. It does not support multiple Horizon replicas or arbitrary external DME servers. The integrated internal listener retains TCP `10073` and UDP `10474`; forwarding must match the advertised endpoint.

The strict configuration accepts 2–16 entries, unique worker IDs and worker endpoints, capacities up to 64, and exactly two positive team minima. Duplicate and unknown JSON fields, incomplete entries and files larger than 65536 bytes are rejected. Worker profile catalogs must support the requested faction pair, orientation and game mode. Each configured worker must advertise `worker_registry_v1` and acknowledge its scoped identity.

## Placement and ownership

A global human queue prioritizes the longest-waiting faction and randomizes a compatible opponent and orientation. Existing compatible live worlds are filled before an idle worker stages a new match. Team minima and maximum capacity apply per worker. A staged batch is installed as a whole before local formation runs. Bots do not substitute for human startup minima.

Account ownership is global. One account cannot hold live reservations in two worlds. Requeue from a completed match retires its old transport reservation before a new worker admission, while outstanding settlement receipts stay with their original worker and match. Shared durable progression is serialized in Horizon; a committed receipt refreshes the character snapshot of its current owning worker without changing that worker's match XP or replaying rewards.

Each scoped bridge binds one worker incarnation. Requests, subscription frames and receipts must retain that exact identity. Worker restarts are not automatically adopted by an existing host. A lost stream lease or sticky identity fault stops new admission to that entry; healthy workers can continue. This does not provide durable recovery of a crashed worker's lost reward outbox.

## Private parties

Configured retail registries can opt in with `"party_invite_seconds": 30`
at the top level of the JSON. The supported range is 1–300 seconds and
omitting the field leaves party matchmaking disabled. Start at least one
eligible profiled gameplay worker with `--party-reservations`. Its discovery
reply supplies actual per-profile squad capacity and counts for each side.
A worker without those fields can still admit solo players. Standalone
lobbies continue to offer solo matchmaking.

Private invites bind current authenticated accounts and their fixed selected
characters. A party is same-faction and its leader is first in the roster.
Only that current leader can queue or cancel the party entry. After
publication, a cancel releases the original entry's still-unclaimed
reservations and leaves claimed members in the match. A leader leaving
dissolves the party. An invite response carries no generation
on the wire, so a party never reissues an invitation to the same account.
Expiry, departure and capacity are replacement policy. This scope does not
emit the native decline-presentation event or support open parties and voice.

A group counts toward the human startup and team limits as one indivisible
entry. It occupies one authored squad in a new formation. Solo late joins
still use the ordinary placement path. Horizon prepares every selected
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
