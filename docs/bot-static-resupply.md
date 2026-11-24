# Bot-only static resupply

The optional Sabotage provider catalog enables one timed reserve refill for a
dry idle bot at an authored supply point. Load it with
`--bot-resupply-catalog /absolute/SUPPLY.json` alongside a firearm-enabled bot
population. The worker requires retail mode0, the selected source map, and exact
spawn/collision file digests. The option is absent by default.

Catalogs are supported for the three retail Sabotage maps. Each map pins its
level archive and CLV identity, its expected provider count and the SHA-256 of
the private witness that qualified its selection. The tool and the worker share
these values:

| Map | Providers |
| --- | --- |
| `mag_cosmodrome_final` | 3 |
| `mag_kennicott_final` | 2 |
| `mag_remote_comms_final` | 3 |

The worker rejects catalogs for any other map, a source identity or witness
digest from another map, and any provider count other than the pinned one.
Level and CLV digests come from `SABOTAGE_MAPS` in
`tools/extraction/project-retail-sabotage-objectives.py`.

Project private operator data from the owned original level and current runtime
inputs using the maintained tool. `--map` selects the level; it defaults to
`mag_cosmodrome_final`, and the Cosmodrome output is unchanged by the option:

```sh
python3 tools/project-retail-bot-resupply.py \
  --map mag_kennicott_final \
  --level-archive /absolute/mag_kennicott_final.psarc \
  --spawn-catalog /absolute/SPAWNS.json \
  --collision-world /absolute/WORLD.json \
  --out /absolute/SUPPLY.json \
  --receipt /absolute/SUPPLY.receipt.json
```

The spawn catalog and collision world must select the same map in mode0. Add
`--check` to reproduce existing bytes. The supported allowlist selects visible,
unfrozen, unsnapped source placements. That is an explicit replacement
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
routes cannot authorize the walking approach. The separate one-certificate
opt-in below is required for routes that need a drop.

Seeking retains the accepted corridor through the existing physical movement
follower, preserves tactical role and goal, and owns an exact movement token.
It suspends other motion planners and weapon use for that seek while continuing
physics, observation and damage. Cancellation clears only its own motion; a
newer owner's route survives. Normal tactical movement replans from the current
body after the interruption. A complete query or projected endpoint never
starts service: the actual body must settle grounded inside the same trigger
with current equipment, life, allocation and fresh quiet. Service time begins
at that separate Start, after walking ends.

An optional `allowed_drop_id` in the approach parameters enables one certified
passage on the same pinned mesh. Its value is an external positive certificate
ID, paired with explicit `planning_window_ms` of 1,000–10,000 ms and no longer
than the seek deadline. The retry interval must be shorter than this planning
window to allow a separate drop query after a failed walk. Omit both fields to retain walking-only behavior. The worker rejects an
unknown certificate and requires an autonomous population, configured drop
policy and explicit fall-damage rules before opening logs. Generated policies
and receipts retain the same source bindings.

An idle, grounded, fully dry bot can first acquire a recorded planning window
before the tactical planner chooses a new route. This is pending planning,
with no passage or refill authority. Normal perception must still supply fresh
quiet and the bounded walking/drop queries must produce a complete qualified
route before movement starts. Deferred perception grants no quiet. Threat,
damage, equipment/scope change or expiry ends the window; its retry cooldown
leaves tactical planning a real turn. Existing active routes remain owned by
their current action.

Drop-capable seeking reuses the normal queue, permit and certified physical
controller. A stable supply parent owns trusted holding, launch and egress
moves without changing the bot's tactical objective. A frozen waiting pose is
not arrival. On interruption before commitment, the bot suppresses launch. If
already committed or still occupying the passage, it retains the motion needed
to clear or recover from that passage; it cannot begin service while cancellation
is pending. A second required drop is outside this opt-in.

After actual clearance, a separate bounded query plans a complete walking
continuation from the current physical body to the saved provider endpoint.
Only subsequent settled, grounded trigger membership with fresh quiet permits
service. Walking, drop survey and continuation queries all consume supply query
credits; a one-credit policy can alternate failed walking and enabled drop
surveys across retries. The option does not authorize taking over an unrelated
active movement.

Human020b interoperability, native packed target identities, non-Sabotage
maps and native animation/presentation remain separate. Internal provider
keys identify exact source instances and are not emitted as native handles.
No human account, resupply XP, UI state or use packet is fabricated. Native
live-match review and release cutting remain the maintainer's responsibility.
