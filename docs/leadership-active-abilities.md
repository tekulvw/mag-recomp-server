# Leadership active abilities

Workers configured for source-backed Acquisition or Domination gameplay enable
the mode's supported squad and platoon command effects by default. The server validates the external ability
inputs before it can accept a match. Authenticated command requests use the normal
Engine event, world tick, damage and reliable replication paths. Source checks
are independent of native-client gameplay acceptance.

## Supported scope

The held native squad leader can issue a command for that exact squad. The held
platoon leader addresses the canonical first squad of that platoon, as the retail
client does. Neither a client-supplied address nor a saved leadership level grants
command authority. A command uses the server clock and the round-owned unit timers;
squad and platoon instances have distinct individual timers and share the platoon
gate. Replacing the leader does not reset those timers. A dead character cannot execute. Availability follows the
retail mode switch and authored lists: Sabotage grants no command abilities,
Acquisition uses its attacker128 lists for both sides, and Domination uses its
side-specific256 lists. The local admission cap does not select a different list.

Attacker admission uses the nearest current anti-air objective and its interception
cooldown. A successful intercept publishes enabled-at-start=false and spends the
normal cooldowns. A rejected target, wrong leader or cooling asset spends nothing.
Defenders bypass anti-air admission and respect the current platoon enabled mask.
The runtime has no faction contract-stat entries, so its tactical cooldown modifier
is zero. Development role assignments cannot grant command authority.

ClusterBomb uses the extracted faction/side schedule, seeded scatter, a filtered
vertical world ray for each due sample, and the existing radial damage path. It
stages at most one sample per native update. Character damage has AirStrike source4
and preserves armor, temporary protection, incapacitation, death and reward rules.
Ground and configured Domination helicopter damage use their existing armor paths.
The invocation keeps its team after disconnect, and a replacement occupant cannot
receive its XP. Qualified structural damage triggers have a separate native
receiver and ordinary objective lifecycle; see [structure strike damage](structure-strike-damage.md).
The shipped Acquisition/Domination maps contain no eligible damage-trigger bodies,
and their hospital bodies remain excluded from Cluster Bomb damage.

Attacker Sensor scans one defender squad per update. Defender Sensor enables the
actual authored platoon objective rosters, whose four-slot scans wrap at the current
Character allocation count. A target needs two distinct accepted movement samples;
re-reading one cached sample cannot establish speed. Detection lasts five seconds
and is replicated through native Character fields. These replacement worlds have
no satellite-jammer producer. Adding one requires its native suppression predicate.

Native platoon election fills vacancies before squad election. It scans the actual
squad/member order, applies the projected platoon weights and nomination bit, and
keeps the first candidate on equal scores. A squad leader can be promoted; that
promotion vacates the squad role and allows its replacement in the same recorded
reconciliation. Withdrawal, departure and transfer remove stale authority. Saved
records and account progression are not modified by holding a role.

Precision Strike admits only an enemy Structural demolition objective from the
qualified map inventory. At its due time it resolves the sealed native target ID
again and requests Completed through the normal objective state/callback path.
It does not apply generic blast damage or make hospitals eligible targets.
Both modes run the qualified Structural entry callbacks for each intermediate
state and publish the final state once. Acquisition profiles with the optional
Structural provider also apply sourced explosions, periodic area damage,
sequence collision, tactical availability and linked spawn changes. Ordinary
demolition and repair interactions for those structures remain separate work;
see [Acquisition gameplay](acquisition-gameplay.md).

Guided Artillery emits a single point blast at the accepted location, using its
vehicle damage channel and its own overlap/cover rules. It does not apply that
channel's amount to characters or objective structures. Strafing Run derives its
three paths from the canonical squad's first authored Alternate spawn, retaining
native pass timing, direction changes, scatter and vertical impact casts.

Gas Bombardment combines separately scheduled bomblets with a persistent chemical
damage volume. The volume uses native half-second pulses, per-life damage admission
shared across overlapping clouds, chemical modifiers and source attribution. Gas
passes through cover; it does not bypass temporary protection or turn a reused
player slot into the original attacker. If the original gas source no longer
resolves, its receiver uses the native unresolved-source damage base.
The current shared collision collector excludes mounted characters. Exact damage
to an exposed vehicle occupant requires the retail seat-visibility binding and
remains unqualified; the vehicle itself still uses its supported damage path.

Domination supports native OIC vacancy election and all five strategic consumers
when configured with the complete projected rules and schema-4 role policy.
Company election precedes platoon and squad election, and promotions refill the
vacated lower roles in the same reconciliation. The authenticated Company leader
can issue an available strategic using the server clock. Sabotage and Acquisition
do not gain OIC authority from the policy. See [OIC strategics](oic-strategic-abilities.md)
for effects, publication and reinforcement semantics.

Configured squad leaders can also assign and cancel manual FRAGO tags and map
waypoints through the shared mode assignment path; see
[manual squad orders](squad-manual-orders.md). Leadership loss clears those
orders without resetting their shared set timestamp.

## Configuration and extraction

Place `<map_name>--active-abilities.json` beside the Acquisition or Domination
gameplay configuration. `map_name` is the selected spawn catalog's map name.
Each setup requires its matching collision world and objective catalog. Workers
must supply `--leadership-passives` and `--leadership-role-policy` for native
squad and platoon election without development bindings. Full effect setups require
the projected schema-3 role policy, or schema 4 for Domination OIC support;
the local fallback record still controls only
characters whose saved record is absent, and is not an account grant. Startup fails if any required
input is missing or invalid.

To use a different absolute manifest path, pass `--active-abilities` on a fixed
Acquisition worker or set `active_abilities` on the applicable formation profile.
See [worker profiles](worker-match-profiles.md).

An ability manifest has this shape, with all paths absolute:

```json
{
  "format": "mag_active_abilities_v1",
  "rules": "/srv/mag/data/ability-rules.json",
  "effects": "/srv/mag/data/ability-effects.json",
  "map_bindings": "/srv/mag/data/map-ability-bindings.json",
  "static_ray_receipt": "/srv/mag/data/static-ray-receipt.json"
}
```

Use `tools/extraction/project-retail-active-ability-effects.py` with the original
retail rules archive to extract schema-2 effect tuning and its six source members.
The complete inventory has 33 rows: six side/faction rows for each effect except
Precision Strike, whose three faction rows are shared by both sides. Reproject map
bindings as schema 3 to include the explicit precision-target inventory alongside
the damage-trigger inventory. Legacy squad-only inputs retain their narrower
capabilities with the squad-only role policy. They cannot be paired with platoon
election, including inside a mixed registry; reproject every affected profile
before using the full platoon policy. Missing platoon data is never supplied
implicitly. Schema-4 Domination setups also require the projected strategic
spawn modifiers; legacy strategic inputs fail startup rather than granting an
OIC with incomplete consumers. The static-ray projector binds the
existing complete source geometry projection to native root and child filters and
records the exact scene hash. Map bindings require the original level archive and
qualified objective allocation/pose catalogs. The populated catalogs and original
files are not distributed with the public server.

Startup checks the original source hashes, matching map and mode, extracted delay,
complete side/faction effect rows, exact current objective catalog and exact
collision scene. Selection preloads everything, so no file access occurs while
switching a formation. `--validate-catalogs` checks these inputs without starting a
worker. A new round gets fresh ability state; retrying an identical selection does
not reset a live runtime.

The ray scenes retain the established reference transforms for parent-attached
geometry. They do not establish complete animated-parent fidelity. The current
replacement movement samples are Engine-owned accepted positions, rather than an
independent native movement simulation.

## Transactions and verification

Command candidates are installed only after durable event recording. Timed effects
are captured with the enclosing world transaction, including actor damage, rewards,
scan cursors, persistent gas volumes, per-life pulse admission and timers. An unavailable collision prerequisite rejects that
candidate and leaves the pending sample available for a later qualified retry.

Each recipient receives platoon creation before later ability deltas. Ability data
is sent with its rising active mask, and late join receives current active data.
Publication retains each object's last queued state, batches object updates, and
defers a recipient when its reliable FIFO lacks room. A failed publication record
changes neither that baseline nor the FIFO. These source checks do not establish
native client effect presentation, exact late-join wake behavior or live acceptance.
Releases remain a human decision after live-match review.
