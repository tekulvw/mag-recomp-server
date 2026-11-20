# Structure strike damage

Squad abilities load by default for supported source-gameplay profiles. Structure
admission follows the retail blast collector and authored physical bodies.

## Retail target rules

| Physical user-data family | Retail role | Cluster Bomb handling |
| --- | --- | --- |
|12|Mission objective|Excluded by the collector|
|13|Defensive emplacement / hospital|Excluded by the collector|
|4|Planted mine|Mine damage and detonation; separate from structural damage|
|56|Objective trigger|Only authored damage or airstrike trigger bodies participate|

Network object type alone does not establish collider identity. The original
hospital physics construction writes its actual family13 identity into the body.
The structure receiver instead resolves a family56 trigger linked to an objective.
Only raw trigger types4 and5 create the native simple-shape phantom. Interaction
volumes of other types do not become damage targets.

All six shipped Acquisition/Domination maps have **zero eligible damage-trigger
bodies**. The original three training maps have eight type4 damage triggers each;
no placed type5 airstrike-only trigger was found across the fifteen retail levels.
Enabling the receiver preserves those authored vulnerabilities. It does not make
C4-only PvP structures vulnerable to Cluster Bombs.

## Receiver and lifecycle

Qualified inputs provide the logical objective/trigger slot, authored threshold,
initial activity and linked box. The box uses the original half extents and a
separate0.05 convex radius on layer6. Its centre supplies the radial test point.
After positive radial eligibility, the receiver accumulates the full raw channel6
damage. This path does not apply actor falloff multiplication, armor, friendly-fire
scaling or the actor cover query. Airstrike-only receivers require source4.
An inactive owner or trigger ignores damage.

Crossing the threshold happens once. If the original source still resolves and
the Structural owner is Deactivated or Repaired, the owner synchronously enters
Activating with its ordinary source-modified arm timer. Range, team, presence and
held input do not govern that timer. Normal Activated/fuse/Completed processing
then dispatches the existing class, collision, callback, spawn and round effects.
There is no direct write to objective health or fabricated strike reward.

The admitted synchronous transition replaces an unstarted queued script target,
as the retail setter does. A callback rejection keeps the crossed threshold and
phantom; future hits do not invoke the owner again. Acceptance removes the
phantom. Activate, Deactivate and objective repair do not invent a damage reset
or recreate a removed phantom. A fresh round rebuilds the runtime.

## Configuration and verification

Reproject ability-map inputs to schema2. The `damage_triggers` array is mandatory,
including an explicit empty list on the supported retail maps. Old schema1 data
fails with a reprojection error. Nonempty rows require a qualified Structural
objective and valid trigger slot. The current nonempty-owner consumer is the
Domination native objective lifecycle; unsupported Acquisition custom rows fail
clearly. [Projection instructions](active-ability-map-bindings.md)

Damage, phantom state, owner transitions, active strike ordinals and publication
are part of the recorded world candidate. Query or recording failure restores
that candidate. Synthetic Engine checks exercise the authenticated command,
timed partial damage, threshold crossing, autonomous timers, ordinary completion,
replaced source, absent source position and failure retries. Retail static checks
qualify the receiver and all six empty source populations. They do not establish
native-client presentation or a live-match result. Existing reference-placement
and broadphase-fidelity limits remain unchanged. Releases remain manual after
the human live-match review.
