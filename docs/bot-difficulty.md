# Bot engagement, recoil and route limits

The `firearms` object in an external bot population config separates AI policy
from weapon parameters. Omitted policy fields use these defaults:

```json
{
  "reaction_ms": 800,
  "turn_degrees_per_second": 90,
  "accuracy_per_mille": 300,
  "far_accuracy_per_mille": 100,
  "max_engagement_distance_m": 60,
  "acquisition_fov_degrees": 120,
  "recoil_compensation_per_mille": 250
}
```

Existing explicit values remain explicit. Updating the executable does not
replace an operator's earlier reaction, turning or accuracy settings.

Acquisition requires an enemy inside the horizontal field of view and strictly
inside both the AI engagement distance and the weapon maximum range. Retention
and the final shot recheck range. A retained target still requires turning and
reaction time; no initial acquisition occurs behind the bot merely because the
world has line of sight. The hit-probability ceiling interpolates from near to
far accuracy with distance, and far accuracy never exceeds near accuracy.

Every shot also needs a body intersection along its actual recoil/spread ray.
The existing damage ceiling alone does not manufacture a hit. Both acquisition
and actual shots retain verified collision coverage, world occlusion and body
blocking checks. This model does not damage incidental bodies on a miss or
invent material penetration.

## Authored weapon inputs

Firing bots read the equipped weapon's `reticule_params_stand` from the explicit
raw item catalog. The existing item binding determines the definition; no weapon
table is included in the executable. Startup validates the two factions' default
bot weapons when firearms are enabled. Equipment/life changes reset recoil.

The supported contract uses angular firing-width increments, pitch/side impulses,
angular bounds and return rates, plus movement-width inputs and `kWCO_Add`.
Recovery rates multiply elapsed time. Automatic sustained fire suppresses firing
width and side contraction; pitch return remains active. The controller's
trigger-hold approximation uses its actual recent firing schedule. This is
standing hip fire with unmodified weapon knock; stance transitions, ADS, perk or
attachment knock modifiers, barrel wander and full player camera corrections are
not implemented. Unsupported nonzero `KnockAdd`, zero pitch-return special
handling, other width combination modes and ambiguous tables fail explicitly.

Catalog angles are converted to radians for trigonometry. Cone width is the
maximum off-axis angle; it is not divided by two. `SpreadNormalization` changes
the radial distribution through repeated squaring or square roots. The server
uses deterministic replacement randomness, not the original TLS random state.
The native player path can receive an already-computed final aim vector; this
server-owned ray model does not claim to reproduce every player camera/muzzle
path or apply another spread sample to a received player shot.

AI recoil compensation reduces its residual pitch/side aiming offset. It does
not change the authored recoil constants or remove intrinsic weapon spread.
Audits record distance, accuracy ceiling, recoil offsets, cone width and actual
shot endpoint. Raw catalogs and native-analysis evidence stay external.

## Objective route continuation

Complete queries may describe paths up to 2,048 m while retaining existing node,
waypoint and corridor limits. Registry movement follows at most 192 m per leg.
Intermediate endpoints come from the validated complete corridor, including
interpolation along a walking edge. A leg never ends partway through a certified
drop. The director keeps the final role goal, replans at a checkpoint, and records
`bot_objective_segment_complete` with that remaining intent.

This removes the former 256 m whole-route rejection and the endpoint-height
shortcut for navmesh movement. Physical ground movement keeps its separate local
limits. The actual follower still checks collision, body occupancy, drop
certificates, health and fall damage. Disconnected paths, unsupported endpoints,
query-budget exhaustion and unknown coverage remain failures. No straight-line
fallback is used, and this change does not establish new navigation coverage.
