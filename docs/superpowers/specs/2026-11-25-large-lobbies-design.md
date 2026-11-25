# Retail-sized lobbies: 128-player Acquisition and 256-player Domination

Status: design and plan only. No product code changes in this PR.

## Intent

Live Acquisition and Domination matches should be retail size:

- **Acquisition:** 128 characters, which is 2 sides × 2 platoons × 4 squads × 8.
- **Domination:** 256 characters, which is 2 × 4 × 4 × 8.

Each server keeps at most **32 humans**, at most 16 per side, and bots fill
every other seat. Today
every match is capped at 64 characters: 32 per side, 4 per squad in
Acquisition and Domination. The docs call authored 128-player Acquisition "a
separate milestone" (`docs/faction-matchmaking.md:56-57`, `docs/acquisition.md:23`).

## Goals

- Run one Acquisition world at 128 characters first, then one Domination world
  at 256, each with up to 32 humans (at most 16 per side) and bots in every
  other seat.
- Use the retail squad size of 8, including for the 64-player Acquisition and
  Domination profiles. The squad addresses the client already understands stay
  unchanged.
- Keep every bot-filled world inside one 50 ms tick at p99 on the live node,
  with headroom.
- Keep per-human downstream bandwidth bounded and measured.
- Keep Sabotage and the 64-player profiles' world size unchanged. The 64-player
  Acquisition and Domination profiles move from 4 to 8 per squad (Decision 7).
- Keep each 32-human world's upstream within the 1 Gbps budget.

## Non-goals

- More than 32 humans per world, or more DME ports. Bots need no UDP ports.
- Concurrent 128 and 256 worlds on one worker. A worker still serves one world.
- Making the Rust worker multithreaded. One world stays on one engine thread.
- Changing the retail client. The client is the recompiled retail v1.0 build,
  and these sizes are native to it.
- Retail relevance or culling rules that we have no evidence for. Any rate
  tiering stays an opt-in local policy (see Phase 3).

## Constraints

- Clean-room and public-tree rules: no game data, captures or extracted catalogs
  in this repository. Capacity values come from the private projector inputs
  (`authored_maximum_players` 128 and 256 are already recorded as provenance in
  the spawn catalog `source`).
- Tests are synthetic. Sims use private inputs locally, and their results are
  recorded here only as numbers.
- Releases, release tags and release-PR merges are human actions.
- Live node budget: talos-2da-doc has 15.95 CPU, and about 4.5 CPU is
  unrequested today. The live gameplay worker requests 1 CPU / 512Mi, limit 3Gi
  (`homelab_k8s kustomize/mag/gameplay-rollout.yaml:17-18`). Mode workers
  request 500m / 3Gi, limit 6Gi (`kustomize/mag-modes/values.yaml:137-139`).

## What limits world size today

Line numbers refer to origin/main at 8f0264e.

### Retail client (private research, not product code)

- **Character pool:** 256 slots. The projectile pools (types 16, 17 and 18) also
  have 256 slots each, keyed by the shooter's Character index. 256-player
  Domination therefore fills the client's Character pool exactly: no spare
  index for a dummy, an overlap during reconnect, or a bot replaced late.
- **Squad pool:** 32 slots, addressed as team×16 + platoon×4 + squad. Domination
  already uses all 32. Acquisition authors 16: indices 0-7 and 16-23.
- **Squad membership:** 8 slots in the Squad object, and the native squad
  change accepts a squad holding 0-7 members.
- **Game messages:** nothing in them carries a maximum-players or world-size
  field. Character and Squad object indices are u16, and CharacterInfo
  `client_index` and `squad_index` are u16 (`gameplay/src/replication.rs:150-222`).
- **Message size:** each game message is at most 1397 bytes, the unfragmented
  plugin limit.
- **Priority and culling:** no retail evidence of relevance filtering, distance
  culling or per-client rate caps was found.

Conclusion: the client takes 128 and 256 without changes. Every blocking limit
is on our side.

### Rust gameplay worker

| Location | Today | Effect | Change |
|---|---|---|---|
| `gameplay/src/lib.rs:263` `MAX_MATCH_PLAYERS` | 64 | One constant used for both world size and human limits | Split it in two: world characters (256, the client pool) and humans (32) |
| `lib.rs:846` configure_match | `maximum_players` ≤ 64 | Horizon's human cap | Keep as the human cap |
| `lib.rs:884` | party members ≤ 64 | Human party reservation | Human cap |
| `match_profiles.rs:317` | Domination `squad_capacity > 4` rejected | 256 needs 8 | Allow 8 when the map has 32 squads |
| `match_profiles.rs:350`, `spawn_catalog.rs:411-413` | team ≤ 32, squad ≤ 8 | World size | Allow team ≤ 128 for profiles that opt in |
| `spawn_catalog.rs:264` | ≤ 32 squads per map | Matches the client pool | Keep |
| `admission.rs:14-22` | admission = team_capacity × 2 | Bots and humans share it | Separate the human cap |
| `projectile_replication.rs:130`, `firearm_assists.rs:194`, `acquisition_character_effects.rs:283` | index < 64 | Range checks on Character index | Use the world-characters constant (256) |
| `progression.rs:20` | rewards sized from 64 | Scales with the constant | Use the human cap |
| `bot_registry.rs:735` | bot total 2..=256 | Already fine | Keep |
| `bot_registry.rs:67` | ≤ 256 population slots | Exactly enough for Domination | Keep |
| `bot_names.rs:4` | 256 names | Enough for 255 bots | Keep |
| `actor_collision.rs:5` `MAX_BODIES` | 256 | 256 characters plus vehicles overflows it | Raise, for example to 512 |
| `multiplayer.rs:8` `MAX_SHARED_QUEUE` | 512 frames per recipient | A joiner gets one snapshot per bot (255) plus live deltas | Scale with world size |
| `multiplayer.rs:1223/1230`, `match_flow.rs:340/384`, `match_cohorts.rs:373` | 128 messages and 24,000 bytes per reply, **for all recipients together** | Hard ceiling on replication; see Network | Structural |
| `bot_perception.rs:62-64` | 32 sight rays and 64 sounds per tick | Global; per-bot freshness falls 4× at 256 | Scale with population |
| `bot_firearms.rs:42` | 64 rays per tick | Same | Same |
| `bot_objectives.rs:229` | `route_queries_per_tick` ≤ 4 (validation maximum) | Same | Same |
| `headless_sim.rs:25` | sim configures 64 | Sim cannot exceed 64 | Phase 0 |

Bot movement publishes one replication packet per bot per 250 ms
(`bot_objectives.rs`, `update_ms: 250`) and broadcasts it separately to every
ready recipient (`bot_registry.rs:1132-1146`).

### Horizon (Medius / DME)

Horizon uses one number for both "world size" and "humans". To reach 128 and
256 with at most 32 humans, it needs a human cap that is separate from world
capacity. The DME slot arrays, the port budget and the matchmaking `ulong`
masks then stay as they are.

| Location | Today | Effect |
|---|---|---|
| `horizon/Server.Medius/Medius/GameplayMatchProfileCodec.cs:41` | team ≤ 32, maximum ≤ 64; one bad row rejects the whole hello | Validation |
| same file `:55` | `bot_fill_total` ≤ maximum | Validation |
| `MagFactionMatchmaking.cs:203-213` | profile maximum ≤ 64, team ≤ 32 | Validation |
| `MagFactionMatchmaking.cs:68,89,109` | the world's team capacity used as the human per-side cap | Needs the human cap |
| `MagFactionMatchmaking.cs:124-154` | `ulong` bitsets over the per-side human count | **Silently wrong above 62.** Stays safe only while it counts humans (≤ 32) |
| `MagWorldPlacement.cs:47-79`, `LocalDmeHost.Matchmaking.cs:214-365`, `LocalDmeHost.Parties.cs:39-146` | room and side checks from profile maximum and team | Need the human cap |
| `MagMatchStartupPolicy.cs:16`, `MagWorldRegistryConfig.cs:84-87`, `ServerSettings.cs:74-75`, `LocalDmeHost.cs:141-147`, `Server.Dme/Models/World.cs:113` | human maximum ≤ 64 | Fine at 32 humans |
| `GameplayBridgeResponse.cs:95-97,152` | 1397 bytes per message, 128 messages per reply, 24,000 payload bytes per reply, 131,072 bytes per response | **Replication ceiling** |
| `GameplayBridgeResponse.cs:240` | party squad must satisfy `unit/16 == side` | Fine: 16 squads per side |
| `LocalDmeHost.cs:1009-1032`, `Config/ServerSettings.cs:99` | DME output flushed once per Medius tick (`TickRate` 10, so 100 ms) | Doubles a bot update's latency to the client |
| `MagDmePortBudget.cs:18-20` | one UDP port per human, shared across worlds | Fine: bots use none |

The GameInfo and world-list wire fields the client reads
(`MagServerStatusResponse` 0x024c) carry human counts as `uint`. They have no
max-players field.

### Projectors, data and deployment

- `tools/extraction/build-retail-spawn-catalog.py:236,247` writes
  `team_capacity` 32 and `squad_capacity` 4 for Acquisition and Domination, with
  the note "Local maximum64/team32/squad4".
- The private population composer already takes a per-squad slot count. With
  8 per squad it filled every squad on the maps tried:
  - 128 slots on an Acquisition map;
  - 256 slots on each of two Domination maps;
  - minimum separation of 1.0 m or more, and no short squads.

  On one Acquisition map, two squads have no spawn that reaches an objective.
  That is true today as well.
- Authored squad bindings exist for every squad: 16 in Acquisition and 32 in
  Domination (`docs/domination-map-metadata.md:41-46`, `docs/acquisition.md:45-62`).
- Chart schema: `gameplayWorkers[].maximumPlayers` ≤ 64
  (`charts/mag-recomp-server/values.schema.json:725-733`).
- homelab:
  - `kustomize/mag-modes/registry-team-minimum.yaml:21,33,45` sets
    `maximum_players` 64;
  - live sets `--bot-fill-total 64` (`kustomize/mag/values.yaml:72-73`);
  - 32 UDP ports are forwarded (`terraform/mikrotik/mag.tf:7`, `kustomize/mag/gateway.yaml`).

## Measurements

Method:

- A throwaway local patch (not committed or pushed) changed the worker as
  follows:
  - raised `MAX_MATCH_PLAYERS` to 256;
  - let Domination use squad 8;
  - let the headless sim select a match profile;
  - overrode team and squad capacity from the environment;
  - recorded each tick's wall time, per-subsystem time, and every packet queued
    for the observer.
- Each run simulated 900 s on virtual time, single-threaded, on an Apple M4 Pro.
  Runs went one at a time under `nice -n 15`, with `CARGO_BUILD_JOBS=2`.
- Statistics exclude the first 120 s.
- One synthetic observer never spawns. Every other character is a bot.

Inputs were the private Acquisition (Panama) and Domination (Unalaska) bundles
from 2026-11-24. The 128 and 256 populations were recomposed privately with 8
slots per squad. Capacity was overridden at load, and the spawn catalogs
themselves still say 32/4, so the behaviour counts below are only indicative.
The headless observer never drains, so these runs do not measure the drain and
fan-out cost for 32 recipients; Phase 0 adds that.

| World | Bots alive (mean) | Tick mean | p50 | p99 | p99.9 | max | Ticks over 25 ms | RSS |
|---|---|---|---|---|---|---|---|---|
| Acq 64 | 61 | 0.62 ms | 0.57 | 1.76 | 6.1 | 10.5 | 0 | 589 MB |
| Acq 128 | 126 | 1.20 ms | 1.11 | 2.62 | 5.1 | 11.3 | 0 | 591 MB |
| Dom 64 | 61 | 0.87 ms | 0.80 | 3.07 | 4.3 | 10.5 | 0 | 883 MB |
| Dom 128 | 125 | 2.01 ms | 1.61 | 5.61 | 7.3 | 10.2 | 0 | 883 MB |
| Dom 256 | 248 | 5.38 ms | 4.62 | 10.88 | 13.3 | 15.2 | 0 | 882 MB |

Mean per-tick cost by subsystem, in ms. "Bots" includes route planning and
drops. P99 is in brackets.

| World | perception | bots (all) | of which route planning | firearms | objectives | other |
|---|---|---|---|---|---|---|
| Acq 64 | 0.009 | 0.32 (1.44) | 0.11 (1.26) | 0.03 | 0.21 | 0.04 |
| Acq 128 | 0.021 | 0.56 (1.94) | 0.21 (1.35) | 0.05 | 0.52 | 0.05 |
| Dom 64 | 0.009 | 0.43 (2.62) | 0.23 (2.40) | 0.03 | 0.36 | 0.04 |
| Dom 128 | 0.029 | 1.12 (4.67) | 0.69 (4.28) | 0.15 | 0.66 | 0.04 |
| Dom 256 | 0.099 | 2.59 (8.00) | 1.56 (6.89) | 0.24 | 2.39 (2.73) | 0.05 |

Findings:

- **CPU is not the binding limit.**
  - Domination 256 has a mean of 5.4 ms and a p99 of 10.9 ms, with no tick over
    25 ms. That is about 11% of one M4 Pro core.
  - Allowing roughly 2× for a slower node core and 32 humans' event handling on
    the same engine lock, one worker still fits in about 1 CPU.
  - Memory is dominated by static world data and does not change with bots:
    about 590 MB for Acquisition and 880 MB for Domination.
- **Cost grows faster than linearly in Domination.**
  - The tick mean goes from 0.87 ms at 64 to 2.0 ms at 128 to 5.4 ms at 256.
  - Objectives cause most of the growth: 0.36 → 0.66 → 2.39 ms, which is
    3.6× for 2× the bots. They look like a per-character × per-objective scan.
  - Route planning causes the tail, with a p99 of 6.9 ms. It is bounded by its
    soft 8 ms slice budget.
  - Both are optimisation targets. Neither blocks 256.
- **Per-tick bot budgets dilute.**
  - Perception sight rays, firearm rays and route queries are fixed per tick,
    for the whole world.
  - At 128 the sims showed more drop-holding and fewer shots per bot than at 64:
    Acquisition 3.8k → 5.3k holds and 7.7k → 5.3k shots; Domination 22k → 41k
    holds.
  - So bots react and route more slowly as the world grows. This is a
    behaviour-quality risk, not a CPU risk.
- **Replication demand per human, as measured.**
  - 70-120 packets/s and 2.2-3.6 KB/s at 64-128.
  - 338 packets/s and 10.4 KB/s at 256.
  - The mean packet is 31 bytes.
  - These numbers are low because most sim bots are holding or waiting and so
    publish nothing.
  - The bound that matters is every bot moving: 4 updates/s per bot.

### Phase 0 baseline: 32 observers at 64 characters

Measured with the Phase 0 sim tooling (#266, branch
`feat/gameplay-sim-profiles-observers`) on the same private Acquisition
(Panama) and Domination (Unalaska) 64-player profiles:

- 900 simulated seconds, statistics exclude the first 120 s, Apple M4 Pro,
  `nice -n 15`, one sim at a time.
- 32 observers drain their queues like clients, 16 per side. The fill total
  counts them, so 32 bots remain.
- Observers send no owner updates, so other humans' relayed updates are not
  included. Byte counts are plugin payload, without RT, DME or TCP framing.
- "Events" is the simulated client event rate per observer. Each event's reply
  runs the shared drain once more, as it does live. At 0, world frames are the
  only drain.

| World | Observers | Events | Bots alive | Tick mean / p99 / max (ms) | Drain mean (ms) | Per observer pkt/s | Per observer B/s | Queue depth max | Evicted |
|---|---|---|---|---|---|---|---|---|---|
| Acq 64 | 32 | 10 Hz | 31.4 | 0.60 / 1.62 / 9.9 | 0.11 | 37.7 | 1,150 | 0 | 0 |
| Acq 64 | 32 | 5 Hz | 31.4 | 0.61 / 1.31 / 11.1 | 0.11 | 36.8 | 1,121 | 0 | 0 |
| Acq 64 | 32 | 0 | — | — | — | 80.0 sent, 102.5 offered | 2,778 sent | 508 | all 32 at 22.5 s |
| Dom 64 | 32 | 10 Hz | 30.2 | 1.19 / 4.11 / 9.7 | 0.17 | 65.9 | 2,027 | 0 | 0 |
| Dom 64 | 32 | 5 Hz | 30.5 | 1.08 / 3.88 / 7.2 | 0.15 | 61.2 | 1,883 | 0 | 0 |
| Dom 64 | 32 | 0 | — | — | — | 79.6 sent, 108.2 offered | 2,930 sent | 508 | all 32 at 17.7 s |
| Acq 64 | 1 | 10 Hz | 62.2 | 0.66 / 1.40 / 11.4 | 0.01 | 52.9 | 1,618 | 0 | 0 |
| Dom 64 | 1 | 10 Hz | 61.4 | 1.13 / 4.39 / 11.2 | 0.02 | 78.3 | 2,405 | 0 | 0 |

Findings:

- **Steady per-human demand is small.** With 32 bots it is 38 packets/s and
  1.2 KB/s in Acquisition, and 66 packets/s and 2.0 KB/s in Domination. The
  mean packet is about 31 bytes. All 32 observers together receive 37-65 KB/s
  (0.3-0.5 Mbit/s).
- **The bot population start is a burst.** In the first 20 s each observer
  receives 102-108 packets/s: a creation snapshot per bot, then first
  movement. That is above the 80 messages/s per human that world frames alone
  can carry at 32 recipients.
- **World frames alone cannot carry 32 humans, even at 64 characters.** With no
  client events, every observer's queue reached 508 and all 32 were evicted as
  `replication_backlogged` within 18-23 s. The world then emptied of bots.
- **Event replies carry most of the drain live.** At 5 or 10 events/s per
  observer, queues peaked at 40-86 messages during the start and were empty in
  steady state. The real client event rate is not measured yet; a client that
  sends few events while loading is the one at risk.
- **Fan-out CPU is cheap.** Draining to 32 recipients costs 0.11-0.17 ms per
  tick. The 320 simulated reply drains per second cost 0.21-0.24 ms per 50 ms in
  total. Tick cost with 32 observers matches 1 observer.

## Network

Per human, worst case at Domination 256 (255 other characters):

| Source | Rate | Bytes | Per human |
|---|---|---|---|
| Bot movement (224-255 bots × 4 Hz, 31-byte packets plus about 5 bytes of RT framing) | 900-1020 msg/s | ~36 B | ~32-37 KB/s |
| Other humans' updates relayed by the worker (31 × client rate, assumed 10-20 Hz) | 310-620 msg/s | ~40-60 B | ~12-37 KB/s |
| Shots, damage, deaths, objectives, stats | — | — | ~5 KB/s |
| **Total** | **~1.2-1.7k msg/s** | | **~50-80 KB/s (0.4-0.65 Mbit/s)** |

- 32 humans need about 1.6-2.6 MB/s (13-21 Mbit/s) of server upstream, before
  TCP/IP overhead. Coalescing (Phase 2) roughly halves that, because the 19-byte
  replication header dominates a 31-byte bot packet.
- DME applies no per-client rate cap or throttle. `ClientObject` writes the
  whole TCP queue in each flush (`horizon/Server.Dme/TcpServer.cs`).

The hard limit is the worker-to-Horizon bridge, not the DME:

- Each world frame and event reply carries at most 128 messages and 24,000
  bytes, **shared by all recipients** (`GameplayBridgeResponse.cs:95-97,152`;
  `multiplayer.rs:1223-1230`).
- World frames run at 20 Hz, so they deliver at most 2,560 messages/s and
  480 KB/s in total.
- At 32 humans that is 80 messages/s per human, against a demand of roughly
  1,000-1,700.
- Event replies add capacity in proportion to the clients' event rate. Each one
  is a separate bridge TCP connection under the single engine lock.
- Queues then fill past 512 frames, and recipients are evicted as
  `replication_backlogged`.
- With every bot moving, this already bites at 64 characters once about 10
  humans are present (63 bots × 4 Hz × 10 = 2,520 msg/s). At the sims' measured
  demand, it bites at about 25.
- Bridge JSON is about 160 bytes per message, carrying a hex payload plus the
  reservation id. Sent as one message per recipient per packet, 32 humans would
  mean 5-8 MB/s of JSON to parse in Horizon.

Every option below must therefore change how replication is framed. Raising
the constants alone is not enough.

## Options

**A. Raise the constants.** Set 64 → 256 and 32 → 128 everywhere, including
Horizon. This is rejected:

- the bridge ceiling still starves recipients;
- `MagFactionMatchmaking`'s `ulong` masks break silently above 62;
- human admission would accept up to 128 per side, which nothing else can
  carry (32 ports).

**B. Split human capacity from world capacity, then scale replication.
Recommended.**

- Profiles carry world capacity (128 or 256, team 64 or 128, squad 8) and a
  separate human capacity (≤ 32 per world, ≤ 16 per side).
- Horizon admits humans against the human capacity only. The worker owns world
  size and bots.
- Replication:
  - coalesce bot character deltas into multi-object updates per recipient;
  - send shared frames once, with per-recipient sequence numbers;
  - flush DME output on frame arrival, or at 20 Hz.
- Per-tick bot budgets scale with population.

**C. Run Domination at 128 with retail squads.** This is a fallback if 256
fails acceptance on the client or the node: 16 squads per side × 4, or 8 squads
per side × 8. It keeps Character-pool headroom, and the same Phase 1-2 work
applies.

**D. Shard one world across several workers.** Rejected: one world has one
authoritative engine. Shards would need cross-worker physics, damage and
objectives, which is far larger than this work.

## Phased plan

Each phase is one or more focused PRs from current main. Every PR keeps existing
64-player profiles byte-identical in behaviour unless a profile opts in.

### Phase 0: measurement (1 PR)

1. `feat(gameplay): measure tick cost and replication demand in the headless sim`
   - `MAG_SIM_PROFILE`, to select a match profile. This makes Domination
     simulable.
   - `MAG_SIM_OBSERVERS=N`: N ready observers, to exercise fan-out and drain for
     32 recipients.
   - A per-tick JSONL summary: tick time, subsystem time, queued and drained
     packets and bytes per recipient, and backlog.
   - Docs: `docs/headless-simulation.md`.
   - Depends on nothing. Synthetic tests drive a two-observer sim.

### Phase 1: capacity model (3 PRs plus a homelab PR)

2. `feat(gameplay): separate world character capacity from the human cap`
   - Split `MAX_MATCH_PLAYERS` into `MAX_WORLD_CHARACTERS = 256`, which bounds
     Character and projectile index checks, and `MAX_HUMANS = 32`, which bounds
     configure_match, parties and terminal rewards.
   - Spawn catalog and profile validation allow team ≤ 128 and squad 8 for
     Domination with 32 squads.
   - Raise `actor_collision::MAX_BODIES`.
   - Scale `MAX_SHARED_QUEUE` from world size, for example 2 × characters + 256.
   - Hello profile rows gain `human_capacity` (≤ 32) and `human_team_capacity`
     (≤ 16) under a
     new capability `world_capacity_v1`, and `maximum_players` keeps meaning
     world size. Rows without the capability keep today's meaning.
   - Bots allocate Character indices above the human range, so a late human
     never needs a bot retired just for its index.
   - Depends on 1, for sims only.
3. `feat(medius): admit humans against a human cap separate from world size`
   - Codec accepts maximum ≤ 256 and team ≤ 128 only with `world_capacity_v1`,
     otherwise it keeps today's checks.
   - `MagMatchProfile` gains `HumanCapacity` and `HumanTeamCapacity`.
   - These use the human caps: `MagFactionMatchmaking` (cohorts, `GroupedCohort`
     masks, with an assertion ≤ 62), `MagWorldPlacement`, late join, and parties.
   - `bot_fill_total` ≤ world maximum.
   - Depends on 2, for the bridge field contract.
4. `feat(charts): describe world capacity and human capacity separately`
   - Schema and values. `maximumPlayers` stays the human cap, and a new optional
     `worldCapacity` is added for documentation and checks.
   - Depends on 3.
   - Then a homelab_k8s PR, staged later (see Rollout).

### Phase 2: replication scaling (3 PRs; required before any 128 world takes more than a few humans)

5. `perf(gameplay): coalesce bot character updates into multi-object replication packets`
   - Within one tick, append each recipient's due bot deltas into one 0x0007
     update of at most 1397 bytes, and split when full.
   - Each object keeps its own per-field presence bits. There is one sequence
     per packet.
   - The NAK history stores the coalesced packet.
   - Behind `--coalesce-bot-replication`, off by default, until a test-server
     client check passes. Then make it the default for bot-filled profiles, with
     the switch kept.
   - Expected result: about 51 → 1-2 packets per recipient per tick at 256, and
     about half the bytes.
   - Synthetic codec round-trip tests, plus an A/B with Phase 0 tooling.
6. `perf(medius): carry shared replication frames once with per-recipient sequences`
   - A new bridge message shape for shared frames: `{payload, recipients:
     [{reservation_id, player_id, sequence}]}`.
   - Horizon patches the 32-bit sequence. It sits byte-aligned at bytes 2-5 of
     the 0x0007 header.
   - Size the reply budget per recipient, for example 1397 B × 64 per recipient
     per frame, instead of 24,000 bytes in total.
   - Raise `MaximumResponseBytes` to match.
   - Gated by a capability. Old workers and old Horizons keep the current form.
   - Depends on 5 for measurements, but can be built in parallel.
7. `perf(medius): flush gameplay DME output when a world frame arrives`
   - Today output waits for the next 100 ms Medius tick.
   - Measure latency and syscall cost. Keep a switch, and record the A/B in docs.

### Phase 3: bot behaviour at scale (2 PRs)

8. `fix(gameplay): scale bot perception, firearm and planning budgets with population`
   - Express the budgets per bot, with a global CPU guard.
   - Raise the `route_queries_per_tick` validation maximum.
   - Policy defaults keep today's values at 64.
   - Gate: at 128 and 256, sim reaction latency, shots per bot, holding and
     route-wait must stay at or near the 64 values.
   - The same PR, or a sibling `perf(gameplay): ...`, should index the
     objective tick's per-character checks. They grew 3.6× from Domination 128
     to 256.
9. `fix(gameplay): relieve drop and spawn-exit congestion with eight-member squads`
   - Driven by the holding growth seen at 128.
   - May be mostly private data: more qualified exits and staggered first waves.

Optional, later, and only with an A/B:
`perf(gameplay): publish distant bots at a lower update rate`. This is a local
policy with no retail evidence behind it. It is built only if coalesced updates
(Phase 2, item 5) prove insufficient on the test server, and it stays opt-in.

### Phase 4: data and projectors (1 public PR plus private bundles)

10. `feat(tools): project authored squad and team capacity for Acquisition and Domination`
    - `build-retail-spawn-catalog.py` gains `--capacity local64|authored`, and
      the default stays `local64`.
    - `local64` moves Acquisition and Domination from squad 4 to the retail
      squad 8 (team 32, so half the squads are used), per Decision 7. This needs
      the Domination squad-8 validation from item 2.
    - `authored` writes Acquisition team 64 / squad 8 and Domination team
      128 / squad 8, taken from the recorded `authored_maximum_players`.
    - The note in `source` says which policy was used.
    - Private follow-up: rebuild bundles with 8-per-squad populations, profile
      rows with `bot_fill_total` 128 or 256 and `human_capacity` 32, and refresh
      the bot resupply and approach sidecars.

Order and dependencies:

```text
1 ─┬─> 2 ──> 3 ──> 4 ──> homelab (test)
   ├─> 5 ──> 6 ──> 7
   └─> 8 ──> 9
10 (independent) ──> private bundles
Acquisition 128 on test: needs 2, 3, 5, 6, 10, plus private bundle
Domination 256 on test: needs all of the above, plus 8, and the Character-pool checks below
```

Acquisition 128 comes first:

- it leaves 128 Character slots spare;
- it needs half the replication;
- its squad layout (16 squads × 8) already passes the Squad pool.

Domination 256 follows once Acquisition 128 has run clean on the test server.

## Risks

1. **Client acceptance of coalesced multi-object updates.**
   - The format allows several objects per update, but we have not yet sent
     many characters in one packet to the retail client.
   - Mitigation: flag off by default; check on the test server with packet
     receipts; fall back to Phase 6 and 7 alone.
2. **The Character pool is full at Domination 256.**
   - Every index 0-255 is in use, so a human reconnecting, or a bot replaced by
     a late joiner, must wait for the old object's removal before its index is
     reused.
   - Mitigation: reserve indices 0-31 for humans, and bots use 32-255. Add
     tests that pin index reuse to removal. Fall back to Option C if the client
     shows pool exhaustion.
3. **Bridge and Horizon throughput.**
   - JSON volume and the single engine lock scale with humans × characters.
   - Mitigation: Phase 2. Phase 0 measures drain cost at 32 observers before
     rollout.
4. **Bot quality falls at scale.**
   - The sims show more drop-holding and fewer engagements per bot at 128.
   - Mitigation: Phase 3, with sim gates.
5. **Live node CPU.**
   - The sims say one core is ample. The live core is slower than an M4 Pro, and
     32 humans add event handling.
   - Mitigation: Phase 0 tick telemetry on the test server, and a request of
     1-1.5 CPU for each 128 and 256 worker. They may all run at once on
     talos-2da-doc (Decision 5), within the about 4.5 CPU unrequested there.
6. **Upstream bandwidth.**
   - Up to about 13-21 Mbit/s for 32 humans in a 256 world, and about half that
     after coalescing. The budget is 1 Gbps (Decision 6), so this is not a
     binding limit.
   - Mitigation: Phase 0 per-recipient byte receipts.
7. **Matchmaking masks.**
   - `ulong` cohort masks would break silently if a world-size team capacity
     reached them.
   - Mitigation: they count humans only, with an assertion and a test at the
     boundary.
8. **Spawn and drop data quality.**
   - 8 per squad makes spawn clusters denser, and some squads' spawns don't
     reach objectives.
   - Mitigation: population report checks in the private bundle build.

## Test strategy

- **Synthetic (CI):**
  - Rust tests cover:
    - profile and catalog validation at 128 and 256;
    - squad 8 in Domination;
    - human cap versus world capacity, Character-index allocation (bots above
      the human range) and index reuse;
    - `MAX_BODIES` and queue scaling;
    - multi-object replication encode and decode round trips at the 1397-byte
      split boundary;
    - NAK replay of coalesced packets.
  - C# tests cover:
    - codec rows with and without `world_capacity_v1`;
    - matchmaking and placement at 32 humans in a 256 world;
    - `GroupedCohort` at its bound;
    - shared-frame sequence patching and budget enforcement.
  - Chart schema tests.
- **Sims (local, private inputs):**
  - Phase 0 tooling at 64, 128 and 256, with 1 and 32 observers.
  - Gates:
    - p99 tick < 10 ms on the Mac;
    - no `replication_backlogged` evictions;
    - per-recipient bytes within budget;
    - behaviour metrics within tolerance of the 64 baseline.
- **Test server (LAN `test.magtest.online`):** retail clients and portal
  service accounts.
  - First Acquisition 128 with 1, then 4, then 8 humans; then Domination 256.
  - Check:
    - client stability;
    - every bot visible and moving;
    - squad and platoon UI;
    - late join;
    - reconnect;
    - match end and results;
    - tick telemetry;
    - pod CPU and memory.

## Rollout

1. Merge Phases 0-2 and 10 with defaults unchanged. Live stays at 64.
2. Test server: one Acquisition 128 profile through the private bundle and the
   registry, flags on. Then Domination 256. Iterate on Phase 3.
3. mag-modes: add 128 and 256 profiles next to the 64 profiles, raise worker
   CPU and memory requests, and watch `match_configured` and the tick receipts.
4. Live: switch Acquisition to 128, wait at least one week, then switch
   Domination to 256.
5. Make coalescing the default only after the live A/B. Keep the switch.

Each step is reversible by selecting the 64 profiles again. That needs no code
rollback.

## Cost

About 10-11 public PRs:

| PR | Phase | Complexity |
|---|---|---|
| 1 | Phase 0 | small |
| 2 | Phase 1 | medium |
| 3 | Phase 1 | medium |
| 4 | Phase 1 | small |
| 5 | Phase 2 | large |
| 6 | Phase 2 | medium-large |
| 7 | Phase 2 | small |
| 8 | Phase 3 | medium |
| 9 | Phase 3 | medium, mostly data |
| 10 | Phase 4 | small |

Also needed:

- 2-3 homelab_k8s PRs (test, then modes, then live);
- private bundle rebuilds for 6 maps;
- test-server sessions for each mode.

Roughly 3-5 weeks of agent work, with test-server time being the long pole.

## Decisions

Recorded from the user on 2026-11-24.

1. **Acquisition 128 first, then Domination 256.** Domination 256 follows once
   Acquisition 128 has run clean on the test server. Option C stays the
   fallback.
2. **Human cap.** At most 16 humans per side, so 32 per world.
3. **Coalesced replication.** Approved: combined per-human bot updates may be
   tried, opt-in, on the test server first.
4. **Distant-bot rate tiering.** Only if the coalesced updates prove
   insufficient, and opt-in.
5. **Live resources.** The 128 and 256 workers may request 1-1.5 CPU each, and
   they may all run at once on talos-2da-doc.
6. **Upstream bandwidth.** The budget is 1 Gbps.
7. **Squad size 8 for 64-player profiles.** 64-player Acquisition and Domination
   profiles move to squads of 8, matching retail.
