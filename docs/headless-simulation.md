# Headless bot simulation

`mag-gameplay --headless-sim-seconds N` runs a match on virtual time without a
client and exits. It is a local diagnostic for bot behaviour (spawn exits, drop
admission, routing), tick cost per subsystem and per-recipient replication
traffic, against whatever catalogs, collision world, navmesh and bot population
you pass; nothing private is bundled.

How it works:

- The bridge setup Horizon would send is issued in process: `hello`, then
  `select_match_profile` when `MAG_SIM_PROFILE` names a `--match-profiles`
  registry profile (the selection configures the match, as it does live), or
  `configure_match` otherwise, for every human seat the loaded spawn catalog
  admits (64 in a 64-character world, the human cap in a larger one).
- `MAG_SIM_OBSERVERS` observers (default `1`) are reserved as players
  `0..N`. Each is ready for updates and drains its reliable FIFO and
  notification queue the way a connected client does. Its one-time creation
  bootstrap (ground vehicles, helicopters, objectives) is marked delivered,
  because a real client gets it in replies to its own events.
- The first observer is given a body far outside the map only long enough to
  start the first bot population. Every observer then stays in its first-spawn
  wait, so bots never engage them. The fill total counts observers, as it
  counts human reservations: 32 observers in a 64-player profile leave 32 bots.
- Observers alternate between the profile's attacking and defending factions,
  attacking first. `MAG_SIM_FACTION` pins all of them to one faction.
- Observers are kept from idle expiry, but not from backlog expiry: an observer
  whose reliable FIFO fills is evicted as `replication_backlogged`, as it would
  be live, and the summary reports when.
- World frames drain the shared queues once per tick, within the bridge's
  per-reply budget. Live, every client event's reply also drains them.
  `MAG_SIM_EVENT_HZ` (default `0`, at most `60`) adds that: each observer
  contributes that many reply drains per second, spread evenly. The event's
  own processing and direct replies are not simulated.
- The world advances in 50 ms ticks as fast as the CPU allows (roughly 30
  simulated minutes in about 10 s on a laptop) and writes the normal evidence
  log given by `--log`.
- `MAG_SIM_SNAPSHOTS=1` records `sim_bot_snapshot` every 500 ms: each live
  bot's position, drop state and goal. In Acquisition and Domination,
  `in_structure` is true when the bot's standing body penetrates a placed
  structure, and `leaving_room` when it stands in a structure's enclosed room
  or its walls ([bot-structure-collision.md](bot-structure-collision.md)).
- `MAG_SIM_COMPLETE_STRUCTURALS=24@60,27@90` (Acquisition only) completes
  each listed Structural objective (client object index) at that simulated
  second through the same queued request an authored script makes, standing in
  for attackers destroying it. Its authored hooks then run as they would live,
  for example an anti-aircraft structure opening its platoon's parachute drop
  zones. Each request is logged as `sim_structural_completion_requested`, with
  `error` set when the profile has no such Structural objective.

Pass the same arguments the server runs with, plus the flag, for example:

```sh
MAG_SIM_PROFILE=... MAG_SIM_OBSERVERS=32 MAG_SIM_EVENT_HZ=10 \
MAG_SIM_WARMUP_S=120 MAG_SIM_TICKS=/tmp/sim.ticks.jsonl \
mag-gameplay --bind 127.0.0.1:0 --log /tmp/sim.jsonl \
  --application-id 20231 --item-catalog ... --match-profiles ... \
  --headless-sim-seconds 900
```

Without `MAG_SIM_PROFILE`, pass the single-map arguments instead
(`--spawn-catalog`, `--collision-world`, `--navmesh-world`, `--bot-population`,
`--bot-fill-total` and so on).

The run is deterministic for identical inputs, except for wall-clock timings,
so compare policy changes on the same inputs; a single run does not show
variance. Invalid `MAG_SIM_*` values fail the run instead of falling back.

## Measurements

When the run ends, one `headless_sim_summary` JSON line goes to stderr. Ticks
before `MAG_SIM_WARMUP_S` (default `0`) are left out of it.

- `tick_ms`: mean, p50, p99, p99.9 and maximum wall time of `tick_world`, plus
  `ticks_over_25ms` and `ticks_over_50ms`.
- `phase_ms`: the same statistics per subsystem. `bots.population`,
  `bots.drops` and `bots.objectives` (which includes route planning) are part
  of `bots`. The engine records these only after `enable_tick_timing`, which
  only the sim calls.
- `event_drain_ms` and `event_drains_per_s`: cost and count of the simulated
  event-reply drains. They are not part of `tick_ms`.
- `bots_max` and `bots_alive_mean`.
- `recipients`, one row per observer:
  - `sent_packets_per_s` and `sent_bytes_per_s`: messages released to it, and
    their plugin payload bytes, without RT, DME or TCP framing;
  - `offered_packets_per_s` and `offered_bytes_per_s`: what the world produced
    for it, which is sent plus queue growth;
  - `queue_depth`: messages waiting in its reliable FIFO and notification queue
    after each tick;
  - `evicted_at_ms`, when it was evicted.
- `total_sent_packets_per_s`, `total_sent_bytes_per_s` and `evictions`.
- `bot_threads` and `sight_prefetch`: the `--bot-threads` count, and sight
  lines answered ahead (`prefetched`), then used (`hits`) or asked directly
  (`misses`). All zero with one thread.

`MAG_SIM_TICKS=PATH` also writes one JSON line per tick, warmup included:
`t_ms`, `tick_ns`, `phases_ns`, `bots`, `bots_alive`, `event_drains`,
`event_drain_ns`, `sent_packets`, `sent_bytes`, and `observers` as
`[player_id, sent_packets, sent_bytes, queue_depth]` per connected observer.

Useful evidence receipts: `bot_population_spawn` / `bot_respawn` (life starts,
with the spawn point and arrival chosen as in [bot-active-spawns.md](bot-active-spawns.md)),
`bot_drop_admission` (`queued`, `granted`, `convoy_departing`, `released` with
`reason`), `bot_drop_delay_sample` (per-passage phase timestamps) and
`bot_planning_slice`.

## Single-team regional bot allocation

A direct Acquisition worker with an existing source-qualified
`regional_collision` policy may also set `regional_team` to the native team
index `0` or `1` in its bot population file. Every slot must name that team,
its authored squad and an active insertion, alternate or on-foot spawn available
to that squad. Use the exact authored position. The server settles the body
inside the guarded region and checks that any fixed goal is also inside.

`--bot-fill-total` still counts human reservations and bots together. For
example, a fill total of two with one human reservation and one valid regional
slot permits one bot. The bot limit also respects the selected team's physical
capacity, existing squad capacity and the supplied regional slots. The
allocator waits for unavailable slots and does not substitute the other team.
Human reservations retain their ordinary admission, faction and priority.

Omit `regional_team` to keep balanced population behavior. This option does
not supply static movement coverage, create a navmesh or admit an offline
reference artifact. Regional movement still requires its qualified world and,
for route following, a matching bounded runtime navmesh.

## Related diagnostics

- `cargo run --release --example exit_floor WORLD MESH SPAWNS.jsonl` prints the
  free-flow navmesh distance from each spawn position (one JSON array per line:
  `[object_index, team, source_spawn, x, y, z]`) to every drop launch.
- `cargo run --release --example walk_out WORLD MESH "x,y,z" "x,y,z" ...`
  reports, per start/goal pair, whether the start's navmesh island reaches the
  goal on foot, and where a partial route ends.
- `mag-navmesh search-spawn-exits` accepts `seed` per group (probe around a
  point such as a spawn instead of the original drop), `min_radius_steps`,
  `min_launch_spacing_m`, `max_links_per_group`, `precise_overlap` (reject only
  physically exclusive drops, as runtime admission does) and
  `max_health_loss_quarter_points`. Defaults keep the previous behaviour.
  Rejections that used to be silent are counted in `evidence.json`.
