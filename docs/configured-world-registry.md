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

Use the reachable advertised IPv4 address in a real installation. Alternatively set `client_address` to exactly `"$MAG_ADVERTISE_IPV4"`; Horizon then reads the address from its `MAG_ADVERTISE_IPV4` environment variable, with the same validation, so a deployment that keeps that variable current needs no second copy. A drain reload resolves it again and rejects a changed address like any other structural change. The Helm chart's `gameplayWorkers` uses this form (see the chart README, "Configured gameplay workers"). All entries share this client address and TCP endpoint, with distinct registered DME world IDs and reservation tokens, and one pool of DME UDP ports (see [DME port capacity](#dme-port-capacity)). This is one integrated Horizon/DME process with one transport listener and character-store authority. It does not support multiple Horizon replicas or arbitrary external DME servers. The integrated internal listener retains TCP `10073` and UDP `10474`; forwarding must match the advertised endpoint.

The strict configuration accepts 1–16 entries, unique worker IDs and worker endpoints, capacities up to 256 (see below), a total minimum of at least one and exactly two nonnegative team minima that fit the maximum. Duplicate and unknown JSON fields, incomplete entries and files larger than 65536 bytes are rejected. Worker profile catalogs must support the requested faction pair, orientation and game mode. Each configured worker must advertise `worker_registry_v1` and acknowledge its scoped identity.

## Placement and ownership

A global human queue prioritizes the longest-waiting faction and randomizes a compatible opponent and orientation. Existing compatible live worlds are filled before an idle worker stages a new match. Team minima and maximum capacity apply per worker. A staged batch is installed as a whole before local formation runs. A worker without bot fill still forms a new match only with at least two humans, one on each side, whatever its configured minima. A worker that advertises `bot_fill_v1` forms its bot-filled profiles under its configured minima as written, so `"minimum_players": 1` with `"minimum_players_per_team": [0, 0]` lets one human start a match that bots fill on both sides. This is opt-in per worker and per profile; additional humans are placed into compatible live worlds first, as above.

An entry's `maximum_players` is the largest world that worker may form. Up to 64 it is also the entry's human limit, exactly as before. Above 64 it lets the worker offer profiles of that many Characters (advertised with `world_capacity_v1`, see [faction matchmaking](faction-matchmaking.md#world-size-and-human-capacity)); the entry's human limit stays at 64, and each large profile's own caps (32 humans, 16 per side) apply beneath it. A profile larger than its entry's `maximum_players` is not offered on that entry; Horizon logs how many were withheld, and an entry left with no profile fails startup. For example, an Acquisition worker with 128-Character profiles uses `"maximum_players": 128`.

To serve several modes at once, run one worker per mode, each with its own `--match-profiles` manifest and `--worker-id`, and list each worker here. For example, three bot-filled workers for Sabotage, Acquisition and Domination each use `"minimum_players": 1, "maximum_players": 64, "minimum_players_per_team": [0, 0]`. Each mode still advertises every ordered faction pair.

Account ownership is global. One account cannot hold live reservations in two worlds. Requeue from a completed match retires its old transport reservation before a new worker admission, while outstanding settlement receipts stay with their original worker and match. Shared durable progression is serialized in Horizon; a committed receipt refreshes the character snapshot of its current owning worker without changing that worker's match XP or replaying rewards.

Each scoped bridge binds one worker incarnation. Requests, subscription frames and receipts must retain that exact identity. A lost stream lease or sticky identity fault stops new admission to that entry; healthy workers can continue. When a configured worker restarts, Horizon writes off the old incarnation and adopts the new one for that entry only (see "Worker restart adoption" below). This does not provide durable recovery of a crashed worker's lost reward outbox.

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
Expiry, departure and capacity are replacement policy.

The retail invite flow runs as follows. The leader's invite (application type
`0x1A`, carrying party id, invitee account id and inviter account id) creates
the party on first use, answering the leader with joined (`0x1F`) and a roster
(`0x21`), and is forwarded to the invitee. The invitee's answer (`0x1B`) is
read in the client's order: response, inviter, the invitee itself, party id.
An accept sends the joiner `0x1F` and then the roster to every member, leader
first, in the same lobby turn, well inside the client's five-second joining
window. A decline (or the client's error, in-party or timeout answer) is
relayed to the inviter as a decline notice (`0x22`: invitee, party id,
reason). The inviter also gets that notice with reason timeout when an
invitation expires unanswered, with reason in-party when the invitee already
belongs to a party, and with reason error when the invite cannot be delivered
(the target is offline, on another faction, already invited, or the party is
full or queued). Invites whose inviter or party id does not match the sender
draw no notice.

The open-invite toggle (`0x25`: one flag bit, then the party id) is accepted
from the current leader only. It is stored on the party and carried in every
later roster; a change resends the roster to all members. The flag does not yet
change who may invite: retail semantics for it are not established, so only
the leader invites either way.

There is no party voice world. A member's voice rejoin request (`0x26`, sent
after its game connection drops) is recognised and logged
(`reason=party_voice_unsupported`) but not answered; the client then carries on
without party voice. The voice descriptor reply (`0x27`) exists only in the
codec. Party-range types the retail client never acts on (`0x16`, `0x18`,
`0x19`, `0x1D`, `0x1E`, `0x23`, `0x24`) are not parsed and draw no reply.

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
their DME tokens and then continues with ordinary solo admission. A
`reserve_party` that never left Horizon is sent again unchanged. One the
worker did not answer is released by its exact cohort, then reserved again
under a new cohort and new reservation IDs, within the
[gameplay admission](gameplay-admission.md) attempt limit and deadline; a
party whose attempts all fail returns to the queue. A prepublication
failure rolls back the attempted cohorts and ordinary per-person transport
lifecycle applies after publication.

## DME port capacity

The integrated DME gives each connected human their own UDP port, the lowest free one from `10474` upward, shared by every world behind this Horizon and released when the transport closes. A port the router does not forward cannot carry game traffic, so Horizon admits at most one human per forwarded port:

| Variable | Effect |
| --- | --- |
| `MAG_DME_UDP_PORT_END` | Last forwarded UDP port (`10474`-`65535`). The cap is the range size: `10505` gives 32 humans. The DME also hands out no port past it. |
| `MAG_DME_HUMAN_CAPACITY` | Optional lower cap, from 1 to the range size. Alone, it also sets the range to that many ports from `10474`. |

With neither set, the cap is unbounded as before. Invalid values stop Horizon at startup. The Helm chart sets `MAG_DME_UDP_PORT_END` from `network.udpPortRange.end`, and `MAG_DME_HUMAN_CAPACITY` from `network.dmeHumanCapacity`.

Every gameplay reservation, live or showing results, holds one port from admission until its DME transport has stopped. Placement subtracts those ports and the tickets already forwarded to a world from the cap before planning, so new formations, solo late joins and late parties never reserve a human past it. A player who does not fit stays in the global queue and keeps receiving the ordinary waiting updates; no new reply or result is sent. A party needs a port for every member and enters as a unit. While the oldest waiting party does not fit the free ports, players queued after it wait too, so ports freed one at a time go to that party first; players queued before it are unaffected. A party larger than the whole cap can never play together and is refused when queued with the ordinary failure result (2). A reconnecting player queues again and waits like anyone else. If admission still finds every port taken (a race), the world answers its usual full-world result (6), which returns the ticket to the queue. The standalone single-world path applies the same cap to its formation and admission.

Horizon logs the configured capacity at startup (`DME UDP port capacity ...`), `dme_ports=<in use>/<cap>` on each admission, and at most once per 30 seconds `Registry placement held: reason=dme_port_capacity` with the waiting humans and ports in use while players wait only for a port. The admin API's `GET /admin/v1/dme-ports` reports the same numbers (see [admin API](admin-cli.md#admin-api)).

## Worker restart adoption

A configured worker that crashes or is replaced comes back as a new process with a new incarnation ID. Horizon reconstructs that entry's host inside the running registry instead of requiring a Medius restart, which would drop every player in every mode. Old reservations are never rebound to the new incarnation. The steps below touch only that entry; other entries' bridges, worlds, cohorts and admission carry on throughout.

1. **Detect.** Only an entry whose bridge raised its sticky identity fault, or whose host failed to start or tick, is examined. A reply fenced for the bound incarnation that comes back with `worker_identity_mismatch`, or carries another incarnation, raises the fault; the old host's world-stream reconnect produces one within about a second of the new process listening. Once per second Horizon then sends an unfenced `hello` (discovery, which has no effect on the worker) to the configured endpoint on a fresh bridge. If the same incarnation answers, the fault is not a restart: the entry stays closed exactly as before and this is logged once (`Configured worker fault without a restart`). An endpoint that answers without the configured `worker_id` or `worker_registry_v1` is refused (`reason=identity`). An unreachable endpoint is simply probed again. A world stream that breaks and reconnects to the same incarnation never raises the fault, so it is handled as before: the registry releases that world's players through the ordinary world-stream-gap path, resubscribes, and reopens admission without any write-off. (The legacy single-worker host's `world_stream_resume_v1` resume is not used by registry hosts and is unchanged.)
2. **Write off the old incarnation.** Another incarnation answering at the endpoint means the old process and everything it held are gone, so nothing is sent to it. Horizon marks the old cohort terminal, releases every reservation locally with reason `worker_restarted` (the DME seat and player slot are freed and the client's DME connection is closed; normally the world-stream gap has already done this, reason `world_stream_gap`), drops pending remote releases that the old process can no longer acknowledge (so those accounts can queue again at once), clears provisional party holds, returns tickets that had been forwarded to that entry to the global queue, frees the old DME world ID and disposes the old host. Players in the lobby are unaffected; players who were in the match lose their game connection and return to the menu, as on any world-stream gap, and can queue again immediately. The write-off happens whenever a new incarnation is confirmed, even if adoption below is then refused or capped.
3. **Make lost receipts visible.** Horizon keeps every reward and loadout-selection receipt the worker reported until it is settled durably. At write-off each unsettled one is logged once, as `Gameplay receipt lost with worker incarnation: worker=… incarnation=… account=… character=… kind=… amount=… receipt=… match=…`, followed by a summary (`Configured worker incarnation written off: … released_players=… dropped_releases=… lost_rewards=… lost_loadout_selections=… untracked_receipts=…`) and running totals per entry (`Configured worker write-off totals: … lost_rewards_total=…`). A receipt already committed to the character store is never reported lost, even if its acknowledgement to the worker did not arrive. Receipts the worker produced but never reported cannot be known to Horizon and are not listed. Nothing is fabricated or replayed: the worker's reward outbox lives only in its memory, and its `--log` journal is an audit trail rather than a replayable outbox, so lost rewards stay lost and are only made visible.
4. **Adopt the new incarnation.** The new incarnation's discovery reply must equal the entry's established contract: the whole advertised profile matrix (every profile ID, mode, faction pair, capacities, squad layout and bot fill) and the capabilities that shape what the registry offers (`worker_registry_v1`, `match_profiles_v1`, `independent_match_cohorts_v1`, `committed_loadout_selection_v1`, `active_loadout_refresh_v1`, `loadout_layout_revision_v1`, `bot_fill_v1`, `party_reservation_v1`, `party_late_entry_v1`). The contract is the one the entry started with, or for an entry that was unreachable at startup, the one it first starts with. A mismatch refuses adoption with `Configured worker adoption refused: … reason=contract_mismatch profiles_missing=… profiles_added=…`, once per incarnation; the entry stays closed until a matching incarnation answers or Medius restarts. Otherwise Horizon builds a new host for the entry exactly as at startup (scoped identity handshake, capability checks, a new DME world and world stream), checks that the handshake reached the same incarnation with the same contract, and only then installs it and reopens admission for that entry (`Configured worker adopted: … admission reopened for this entry only`). Queued players are placed on the next pass.
5. **Crash-loop cap.** Each entry may start at most `max_adoptions` adoptions within `window_seconds` (default 3 in 600 s). An attempt counts when it starts building the new host, whether or not it succeeds. Beyond the cap the old incarnation is still written off, but the entry stays closed, logging `Configured worker adoption cap reached: … entry stays closed until <time>` once per incarnation, and adopts again when the window allows.

Restart adoption is on by default. To restore the previous behaviour (no write-off and no adoption; a restarted worker's entry stays closed until Medius restarts), or to change the cap, add this optional top-level object to the registry file. Every member is optional; unknown members are rejected:

```json
"worker_restart_adoption": {"enabled": true, "max_adoptions": 3, "window_seconds": 600}
```

`max_adoptions` accepts 1–100 and `window_seconds` 1–86400. The policy is structural, so a drain reload cannot change it. The Helm chart sets it with `worldRegistry.workerRestartAdoption`.

Every adoption creates a fresh DME world with a new world ID and the entry's configured capacity, after the old world has released its seats and ID, so the number of live DME worlds and seats never exceeds the configuration. All entries keep sharing the one advertised TCP endpoint and the integrated listener; adoption opens no ports. Each released reservation returns its DME port lease (see "DME port capacity") exactly once, when its transport has stopped, whether the world-stream gap or the write-off released it; tickets forwarded to the old host stop counting against the free ports when it is written off. Port planning works from the current hosts on every pass and never keys on a world ID, so the adopted world's new ID needs no special handling, and the next match can use the freed ports at once.

Limits: a worker that is down but not yet restarted keeps its released players' remote releases pending, so those accounts cannot queue elsewhere until the new incarnation answers or the old one returns. Two processes answering for one worker ID at the same time (for example a rolling update behind one Service) look like a restart to Horizon; run each worker with a single replica and a recreate update strategy.

## Drain and reconstruction

Replace the configuration file atomically to change `draining` flags. Horizon checks once per second and publishes one complete drain-policy snapshot. Draining blocks new placement and leaves existing world processing active; an adopted host keeps following the entry's published drain flag. Other configuration changes are rejected on reload and require controlled reconstruction. A replacement Rust process is adopted as described above rather than rebinding old reservations to a new incarnation.

## Verification

The normal Horizon check covers strict configuration, placement, exact cancellation, transport ownership, drain publication, world dispatch, terminal transfer, shared progression and restart adoption (write-off of a restarted worker with a lost reward, isolation of the other entry, a same-incarnation stream break, a refused contract mismatch, the crash-loop cap, the off switch and a worker that comes up after startup) using invented loopback workers. The additional check below starts two actual owned Rust processes with invented catalogs and drives Horizon's registry against them, including killing one mid-match and starting it again on the same endpoint while the other's match continues:

```sh
docker build --target check horizon
docker build --target check --file tests/Dockerfile.world-registry .
```

Rust tests separately cover wire fences, subscriptions, restart identity, release-before-reserve ordering and progression-refresh atomicity. These are source and synthetic integration checks; native live-match acceptance remains a separate step. Configuration examples contain no original game data. Source integration does not authorize deployment or release publication.
