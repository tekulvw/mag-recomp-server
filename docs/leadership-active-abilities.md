# Leadership active abilities

Workers configured for source-backed Acquisition or Domination gameplay enable
squad ClusterBomb and Sensor by default. The server validates the external ability
inputs before it can accept a match. Authenticated command requests use the normal
Engine event, world tick, damage and reliable replication paths. Source checks
are independent of native-client gameplay acceptance.

## Supported scope

The held native squad leader can issue a command for that exact squad. A command
uses the server clock and the round-owned unit timers; replacing the leader does
not reset either timer. A dead character cannot execute. Availability follows the
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

Platoon/OIC execution, other command effects and strategic abilities remain
unsupported. Native availability is retained for diagnostics and future consumers;
receiving an unsupported request does not report an accepted activation.

## Configuration and extraction

Place `<map_name>--active-abilities.json` beside the Acquisition or Domination
gameplay configuration. `map_name` is the selected spawn catalog's map name.
Each setup requires its matching collision world and objective catalog. Workers
must supply `--leadership-passives` and `--leadership-role-policy` for native
squad election without development bindings. Startup fails if any required
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
retail rules archive to extract effect tuning. The static-ray projector binds the
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
scan cursors and timers. An unavailable collision prerequisite rejects that
candidate and leaves the pending sample available for a later qualified retry.

Each recipient receives platoon creation before later ability deltas. Ability data
is sent with its rising active mask, and late join receives current active data.
Publication retains each object's last queued state, batches object updates, and
defers a recipient when its reliable FIFO lacks room. A failed publication record
changes neither that baseline nor the FIFO. These source checks do not establish
native client effect presentation, exact late-join wake behavior or live acceptance.
Releases remain a human decision after live-match review.
