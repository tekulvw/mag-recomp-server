# Hospital secondary rocket support

The server has source extraction and library prerequisites for Domination hospital
secondary rockets. This change does not enable firing, apply rocket damage, or
change a running match. Launch admission, the complete live scene adapter,
publication and damage transactions are the next runtime increment.

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
damage paths. These consumers are not enabled by loading this catalog.

## Projectile libraries

Retail replication supports all four registered projectile families across
creation, owner decoding, late-join state and removal. The prototype retains three.
A syntactically valid fourth-family report does not create an admitted Engine
flight or damage source. Existing owner-sequence behavior is retained.

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
policy. It does not certify native collision-agent behavior. The caller must
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

Synthetic tests exercise the profile boundaries, nearest-contact behavior,
initial overlap, margins and filters, deterministic identities, work errors,
straight flight, expiry and source-binding failures. Original extraction and
source inspection are separate from native input and live-match acceptance.
Releases remain manual after the maintainer's live-match review.
