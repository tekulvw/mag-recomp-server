# Headless bot simulation

`mag-gameplay --headless-sim-seconds N` runs a match on virtual time without a
client and exits. It is a local diagnostic for bot behaviour (spawn exits, drop
admission, routing) against whatever catalogs, collision world, navmesh and bot
population you pass; nothing private is bundled.

How it works:

- The bridge setup a medius would send (`hello`, `configure_match`) and one
  reservation are issued in process.
- The reserved observer is marked ready and given a body far outside the map
  only long enough to start the first bot population, then returns to its
  first-spawn wait, so bots never engage it.
- The world advances in 50 ms ticks as fast as the CPU allows (roughly 30
  simulated minutes in about 10 s on a laptop) and writes the normal evidence
  log given by `--log`.
- `MAG_SIM_FACTION` (default `0`) selects the observer's faction.

Pass the same arguments the server runs with, plus the flag, for example:

```sh
mag-gameplay --bind 127.0.0.1:0 --log /tmp/sim.jsonl \
  --application-id 20231 --item-catalog ... --spawn-catalog ... \
  --objective-catalog ... --collision-world ... --navmesh-world ... \
  --bot-population ... --bot-fill-total 64 \
  --headless-sim-seconds 1800
```

The run is deterministic for identical inputs, so compare policy changes on
the same inputs; a single run does not show variance.

Useful evidence receipts: `bot_population_spawn` / `bot_respawn` (life starts),
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
