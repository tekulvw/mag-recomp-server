# Acquisition gameplay

Acquisition is an explicit runtime opt-in. The backend connects timed capture,
the ordered controller, existing Escort activation, driver ownership and motion,
delivery/destruction, scores, results, and a fresh round. Existing profiles stay
unchanged unless they supply the gameplay configuration. Source and synthetic
Engine checks are separate from native live-match acceptance.

## Configuration

For a standalone worker, add `--acquisition-gameplay /absolute/GAMEPLAY.json` to
the explicit retail mode1 startup inputs. For faction matchmaking, put an
absolute `acquisition_gameplay` path in each enabled match-profile entry instead
of supplying that standalone flag. The registry retains fixed character
factions, longest-waiting eligible faction priority and the selected pair's roles.

The strict `mag_acquisition_gameplay` configuration binds the selected map/spawn
catalog, controller, initial objective identity evidence, startup rules and game
rules. It supplies ordered objective/squad inputs, four FSP joins, actual damage
and Arrival geometry, current filter rows, vehicle capabilities/armor and
reinforcement intervals. Controller paths and external data stay external to the
public source repository. Hashes identify supplied inputs; they do not certify
their semantics independently.

Enabled configurations require all four distinct Escort allocations and their
source-matched driver policies, actual compiled map collision, retail combat
metadata, character damage rules, dynamic body/hulk templates and source-bound
static partitions. Missing gameplay damage or Arrival resources reject the
configuration. Startup checks run before logs/listeners; registry checks run
before profile advertisement/selection. `--validate-catalogs` performs offline
configuration checks without opening logs or serving a match.

## Structural objective assets

New source-projected profiles can include the `structural` member in their
Acquisition gameplay configuration. Paths resolve relative to that configuration
file.

```json
{
  "structural": {
    "bindings": "structural-bindings.json",
    "effects": "structural-effects.json",
    "collision_assets": "structural-collision",
    "terrain_index": "terrain-materials.json",
    "terrain_materials": "terrain-materials.bin"
  }
}
```

Use `project-retail-acquisition-structural.py` with the selected map's original
disc, metadata, objective and spawn catalogs, and shared game archive. Use
`--mode GameType128` on `project-retail-domination-structural-effects.py` and
`project-retail-domination-terrain-materials.py` to prepare the matching effects,
collision assets and terrain materials. Keep the projected inputs outside the
public checkout. When `structural` is present, its linked inputs and original
motor-pool allocations must validate before the profile loads. Existing
configurations can omit this member.

The provider runs the sourced startup entries and consumes Precision transitions
in family-12 order. It applies linked insertion points, motor-pool hooks,
tactical command availability, Structural explosions and area damage, and
sequence collision. Committed sequence state is included in late-join
replication. Bound native infantry spawn types 4, 5 and 8 use the authored
category-zero point for local character insertion until their physical routes
are qualified. Ordinary demolition and repair interactions for these structures
also lack runtime coverage.

## Owned gameplay

The objective tick clones objective, ground, character, round and vehicle-wave
authority. Entry effects run before the ordered controller; target requests do
not change the observations used by later branches. Pending state entries and
assignments use the same candidate. One durable record precedes installation
and network release. A failed query, effect or record leaves live authority
unchanged. Completed and subsequent Locked states retain their publication order;
capture holders receive the existing same-generation exit handshake.

Capture activates the exact existing Escort source and clears spawn blockers
through real convex queries. The owner grant uses the authenticated connection
index. Vehicle dynamics require the currently held driver seat, a fresh owner
sequence, operational life and the configured movement budget. Exits at moved
poses use current static support, clearance and dynamic occupancy.

Arrival requires the linked Escort, an admitted current driver, actual chassis
overlap and a positive upward five-metre raw24 ray. A static source witness or
current dynamic root can establish that positive. Lack of a witness is not a
claim about a complete native-world miss. Every occupant needs a supported,
distinct exit before delivery commits. Delivery revokes ownership, enters
vehicle state8 with the configured inert timer, and applies the source objective
award once. The A/B awards in the qualified profiles are600/400 against1000
defending points. Timer expiry moves the delivered vehicle to inert7.

Authenticated firearm reports and grenade detonations use source equipment and
vehicle armor. Fractional HP persists across hits. Critical vehicles decay using
the configured rate. Destruction applies ordinary occupant damage, then the
radial blast, FSP target notification and body/hulk changes. Source-type9 Escort
replacement requires recapture; it does not use the generic reinforcement timer.
APCs(type3) and motor-pool trucks(type7) queue the same allocations at an advanced
vehicle-class wave, enter state6, then respawn at the next due wave. A new match
clears that queue, clock and live vehicle state.

Scores use the native-bound per-update bleed primitive. Depletion publishes the
result once, advances the end deadline and uses the shared terminal/release
path. Late joins receive committed scores, objectives and current vehicle state.

## Scope and verification

Synthetic Engine scenarios cover both deliveries, victory/countdown, release and
recapture; destruction/occupants/recapture; APC/truck reinforcement and restored
human insertion; late joins; rejected routes; and failed-record rollback. Private
offline consumers qualify all six faction pairings across the three maps. These
checks do not establish a native rendered match, throughput or frame rate.

Exact Havok solver/traversal/quaternion bits, native material penetration and
dynamic blast occlusion remain fidelity work. Radial destruction uses an explicit
replacement hard-static-LOS policy. The admitted ground seats are source-bound
closed seats protected from the radial callback; their separate death-occupant
request still applies. Native medals, full objective/vehicle statistics and
cosmetic fades remain deferred. Mines, vehicle-mounted weapon operation and
additional ability effects require their own admitted mechanics. Supporting a
template or decoding a field does not enable those features.

Releases remain manual after the maintainer's live-match review.
