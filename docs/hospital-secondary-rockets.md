# Hospital secondary rocket support

Domination hospital secondaries have optional server-owned launch, straight flight,
contact, ordinary on-foot Character damage and reliable cleanup. They remain off
unless the selected profile explicitly supplies `core.hospital_turrets`,
`core.hospital_rockets` and `core.projectile_contacts`. Direct Engine configuration
and the match-profile registry validate the same source bindings. The CLI's
`--validate-catalogs` path qualifies the weapon rules and contact inputs before
advertising readiness, opening a listener or creating a log.

A current exclusive seat-0 grant, fresh secondary equipment evidence and the
primary readiness fence authorize launch. Cadence and magazine state belong to
the hospital and survive remount. Rockets and frag grenades share native slot
exclusion while retaining separate lifetime state. Owner position and terminal
claims cannot move or detonate an admitted rocket. A flight survives dismount,
death and respawn within its reservation; disconnect/idle removal retires its
wire object before the Character slot can be reused.

## Source-bound external rules

`tools/extraction/project-retail-hospital-rockets.py` consumes an owner's original
retail executable and archives, the item catalog/bindings, reproduced hospital
rules and six profile pairs. It reproduces the hospital source first and refuses
mismatches. The output binds the original ammo identity, default token override,
three faction variants, named muzzle reference hierarchy, weapon timing/ammunition
properties, propulsion/impact strategy, collision sphere and distinct Character,
vehicle and objective blast channels. No game catalogs or binaries are shipped.

The projector accepts `--elf`, `--game-archive`, `--main-archive`,
`--hospital-sources`, `--hospital-turrets`, `--item-catalog`, `--item-bindings`,
`--structures`, six repeated `--profile SPAWNS HOSPITALS`, `--output` and
`--proof-output`. Repeat with `--check` to require byte-identical output.
`hospital_rockets::Catalog::from_bytes` validates the typed output against the
loaded item source, native source identity and exact hospital rules. Hash binding
identifies the supplied source; it is not an independent runtime certification.
Inspect supplied inputs without a listener or game session:

```sh
cargo run --locked --example inspect_hospital_rocket_rules -- \
  RULES ITEMS CHARACTER_BLAST COMBAT HOSPITAL_RULES NATIVE_ELF_SHA
```

The separate authored impact amount does not establish an additional damage
callback. The qualified ordinary on-foot Character route uses its own typed dose,
current body/stance and material cover callback. Vehicle, hospital, Helo, exposed
mounted targets and blocked-ray fallbacks require their respective admission and
damage paths. Ordinary on-foot Character damage is enabled only through an
authenticated, source-bound hospital launch and current contact. The other target consumers
remain separate.

## Runtime and contact policy

The replacement admits untargeted full-power launches with zero inherited
velocity. It derives the origin from the current hospital body and authored
muzzle chain. The normalized request direction must be within a dot product of
0.94 of the recent control basis and its reported origin within 2 m of the
source-derived muzzle. These are replacement admission bounds, not extracted
native tolerances. An accepted request keeps its own direction; it is not snapped
to a lagging aim sample.

Position publication uses a 200 ms local cadence. Contact or age fallback queues
state 2, then inactive removal after a 500 ms minimum opportunity, held while the
terminal is still queued. These intervals are replacement scheduling choices.
A bounded work retry may defer a query for up to 2 seconds; unavailable data or
persistent work failure retires the shot without damage. Queue pressure defers
the candidate while the ordinary reliable FIFO continues draining.

The damage transaction uses the typed infantry dose, current stance/body and
one layer-24 material ray. It commits health, armor, statistics, rewards and the
terminal after one successful record. Exact launch reservation, world, round,
match, Character and team guards prevent reused slots receiving credit. A dead
or respawned source in that same reservation remains eligible for its shot.
Mounted targets, Ground/Helo/objectives, blocked-ray native fallback and rocket
kill-feed weapon indices remain outside this first consumer.

## Projectile libraries

Retail replication supports all four registered projectile families across
creation, owner decoding, late-join state and removal. The prototype retains three.
A syntactically valid fourth-family report cannot create a flight or damage
source. Equipped launch is a separate authority gate. Existing owner-sequence
behavior is retained.

`projectile_contact::sweep_sphere` compares explicitly admitted static/sequence
triangles and current convex roots using one shared work budget. It returns the
earliest contact, sphere centre, optional normal and stable obstacle identity.
Initial overlap is contact even for separating motion. Coincident convex cores
need not have a unique normal. Equal fractions use a stable identity order. Only
the exact associated Character key is excluded; the firing hospital remains an
obstacle. Identity reuse requires the caller to resolve the original reservation
association to the current physical allocation.

`CollisionWorld::projectile_obstacles` enumerates filtered static candidates via
the BVH, including their margins and stable instance/triangle IDs. Its root and
explicit uniform child filter intersection is a represented-world replacement
policy. It does not certify native collision-agent behavior. The scene supplements the
admitted current Sequence and operational hospital/Helo meshes and dynamic solid
roots. The binding pins static and dynamic catalog bytes,
source mesh identities and explicit child/margin metadata. The caller must
supplement every eligible live partition and use the combined collector before
interpreting a miss as clear flight.

Coverage is explicit. `CompleteCurrent` requires declared complete coverage or a
containing verified static region; `ReferencePose` deliberately accepts the
export's represented poses without upgrading its provisional metadata. Missing
metadata, unknown coverage, invalid shapes and work exhaustion are errors. The
current source maps retain some eligible parent-attached compiled roots only at
reference poses. Do not silently relabel them as current or omit sequence,
operational hospital and aircraft overlays.

`projectile_motion::Motion` advances a straight constant-velocity replacement from
an already-qualified launch vector. Each tick returns a detached candidate; the
caller commits only after recording succeeds. Contact produces one terminal at
the interpolated sphere centre. A clear segment may reach the externally supplied
lifetime-plus-grace fallback. Contact wins an exact expiry-boundary tie. The
same-time call is quiet and initial overlap is checked on the first positive
interval. Queue order and retirement remain caller policies, not measured native
frame delays. Query errors and backward time leave the current flight unchanged.

The projectile-only conservative advance divides the query precision budget
between core-distance refinement and its next step, retaining the original
contact ceiling. This avoids rejecting a valid contact for a negative rounding
step while leaving the Character movement solver unchanged.

Synthetic tests exercise the profile boundaries, nearest-contact behavior,
initial overlap, margins and filters, deterministic identities, work errors,
straight flight, expiry and source-binding failures. The six-profile original-data replay covers equipped slot3 launch, live capsule
contact, lethal armor-aware damage, kill credit, terminal/inactive delivery and
reuse, with separate dismount continuity. It uses scripted actor placement and
leaves the separate optional incapacitation catalog disabled. Both six-profile
startup scopes and native input remain separately identified in the evidence.
Original extraction and source replay are separate from native input and
live-match acceptance.
Releases remain manual after the maintainer's live-match review.
