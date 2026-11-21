# Bot-only static resupply

The optional Cosmodrome Sabotage provider catalog enables one timed reserve
refill for a dry idle bot at an authored supply point. Load it with
`--bot-resupply-catalog /absolute/SUPPLY.json` alongside a firearm-enabled bot
population. The worker requires retail mode0, the selected source map, and exact
spawn/collision file digests. The option is absent by default.

Project private operator data from the owned original level and current runtime
inputs using the maintained tool:

```sh
python3 tools/project-retail-bot-resupply.py \
  --level-archive /absolute/mag_cosmodrome_final.psarc \
  --spawn-catalog /absolute/SPAWNS.json \
  --collision-world /absolute/WORLD.json \
  --out /absolute/SUPPLY.json \
  --receipt /absolute/SUPPLY.receipt.json
```

Add `--check` to reproduce existing bytes. The first supported allowlist selects
visible, unfrozen, unsnapped source placements. That is an explicit replacement
policy, not a native activation claim. Original files and generated provider
catalogs are external inputs and are never bundled into public source.

The trigger uses authored translation, nonuniform axis scales and the qualified
X/Y/Z rotation order. It does not substitute the compiled render matrix's
uniform scale. Current grounded physical position must satisfy the inclusive
oriented box test. Tolerance and tick interval come from the catalog. The
geometric representation does not claim PPC SIMD rounding parity.

AI service admission requires a current, grounded, fully dry finite firearm,
idle completed motion and fresh quiet perception, without an objective hold,
seat, drop or traffic operation. Merely overlapping a provider does not refill
ammunition. The recorded Start creates a session with an accepted deadline;
repeated checks retain it. New movement commands pause during service while
idle collision/ground physics still runs.

Fresh enemy visibility, recorded health loss, movement, leaving the box or a
changed life, equipment, allocation, round or provider cancels service. The
scheduler evaluates the service after all shots in that pass, so a later
scheduled enemy hit can interrupt a due pulse. A health-loss revision preserves
that interruption even when healing restores HP before the next check.

One admitted due pulse restores the existing finite bot provisioning reserve
cap and closes the session. Magazine, reload state and the ammo_pool selector
remain unchanged by the pulse. The next ordinary scheduler tick starts reload;
normal weapon timing and field11 publication handle the later shot. Start,
Cancel and Pulse record before installation, and recorder refusal preserves
session and ammunition.

To enable walking to a provider, also load
`--bot-resupply-approach-policy /absolute/APPROACH.json` with the exact matching
`--navmesh-world`. The provider catalog alone retains service at the current
position. Approach admission requires the same dry equipment, fresh quiet and
absence of an objective hold. The optional policy pins the provider catalog's
raw file digest, collision world and consumed Detour payload. Missing or
mismatched dependencies fail before the worker opens its log.

Build private route candidates with the maintained tool and current navigation
CLI. The parameters file is operator policy, with explicit `sample_offsets`,
`projection_radius_m`, `query_budget_per_tick`, `retry_ms`, `goal_radius_m`,
`max_route_length_m` and `seek_max_duration_ms`. Route lengths are positive and
at most 192 m; seek duration is 1,000–120,000 ms, matching the existing movement
limits:

```sh
python3 tools/project-bot-resupply-approach.py \
  --provider-catalog /absolute/SUPPLY.json \
  --mesh /absolute/WORLD.mag-nav \
  --collision-world /absolute/WORLD.json \
  --navmesh-cli /absolute/mag-navmesh \
  --parameters /absolute/APPROACH_PARAMETERS.json \
  --out /absolute/APPROACH.json \
  --receipt /absolute/APPROACH.receipt.json
```

Each policy allows 1–16 route queries per tick and 1–16 anchors per provider.
Projection uses at most 16 explicit sample offsets, each within 5 m of the
authored center. Projection and runtime goal radii are 0.05–5 m; retry intervals
are positive milliseconds. Multiple samples may retain
alternative endpoints on the same walking component; each accepted anchor must
lie inside its referenced provider box. These are route candidates, not proof
of physical arrival. The runtime requires a complete bounded walking corridor,
valid endpoint support and occupancy. Partial, truncated and drop-bearing
routes cannot authorize this approach. Longer spawn-to-provider routes that
need drops remain separate.

Seeking retains the accepted corridor through the existing physical movement
follower, preserves tactical role and goal, and owns an exact movement token.
It suspends other motion planners and weapon use for that seek while continuing
physics, observation and damage. Cancellation clears only its own motion; a
newer owner's route survives. Normal tactical movement replans from the current
body after the interruption. A complete query or projected endpoint never
starts service: the actual body must settle grounded inside the same trigger
with current equipment, life, allocation and fresh quiet. Service time begins
at that separate Start, after walking ends.

Human020b interoperability, native packed target identities, other maps and
native animation/presentation remain separate. Internal provider
keys identify exact source instances and are not emitted as native handles.
No human account, resupply XP, UI state or use packet is fabricated. Native
live-match review and release cutting remain the maintainer's responsibility.
