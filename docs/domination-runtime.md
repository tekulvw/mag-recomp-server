# Domination runtime workstream

The server accepts explicit bounded retail mode 2 profiles for all ordered
faction pairings. The profile generator reads the actual four-platoon map
layout and emits all 32 squad bindings. Local policy caps the match at 64
humans and a squad at four. Authored player bounds remain provenance.

```sh
python3 tools/extraction/build-retail-spawn-catalog.py \
  --disc DISC --game-archive GAME --map MAP --active 0/0/0 \
  --attacking-faction FACTION --output SPAWNS
```

The generator preserves every spawn record and type. The later ground, hospital
and LALO projectors qualify their respective insertion consumers. An authored
Basic point remains the entry fallback when a linked vehicle is unavailable.
Aircraft insertion requires its own qualified runtime; an unavailable Helo point
does not become a fabricated on-foot insertion.

## Initial objective bindings

The Domination projector consumes the metadata catalog and externally qualified
successful fresh-client load evidence. Its source joins include the retail
image, metadata, complete native proof replay, level archive and CLV. The
proof must inventory every placed client objective and exclude trigger
entities from family 12. All 32 squad lists retain authored order and repeats.

```sh
python3 tools/extraction/project-retail-domination-initial-objectives.py --help
```

The output uses the shared ObjectiveCatalog schema with game type 2 and the
native_initial_load scope. That scope allows initial publication and refuses
interaction, demolition, score, reward and post-start rules. Retail profile
owner updates do not invoke the prototype mode 2 elapsed-time countdown.

## Controller commands

```sh
python3 tools/extraction/project-retail-domination-control.py --help
```

The restricted parser converts only the referenced retail callbacks and their
ordered authored parameters. It does not execute Lua. The Rust
`domination_control::Catalog` validates bounded controller data and provides
map-specific event hooks, immediate/delayed invocations and ordered state,
score-rate and command-ability effects. Missing authored callbacks and empty
ability targets are explicit diagnostic records.

`Map::effects` reads the current state supplied by its caller for the entire
invocation. State-change effects are desired-state requests and do not mutate
those observations. The Engine must commit the next entry before a later
callback can observe it. A score-rate request configures the shared native
rate primitive; issuing a request does not immediately deduct points.

## Indexed spawns, triggers and FSP membership

```sh
python3 tools/extraction/project-retail-domination-spawns.py --help
python3 tools/extraction/project-retail-domination-triggers.py --help
python3 tools/extraction/project-retail-domination-fsp.py --help
```

The spawn projection joins original tokens to qualified successful initial-load
family35 indices. It preserves insertion eligibility and positions. The trigger
projection retains each objective's independently proved client/Sim trigger
vector order and authored volume. It does not claim absolute dynamic family56
slots. The FSP projection retains all32 squads' complete source membership and
first category-zero point in source order, including points filtered out of the
initial spawn-selection UI.

`domination_fsp_bindings::Bindings` checks the metadata identity, exact spawn
catalog hash, all point tokens/indices, squad teams and objective links before
planning a committed entry. The FSP planner retains ordered activity updates,
squad fallback/forward selections, and the required linked vehicle hook. Its
spawn stage operates on a clone and exposes the candidate only after vehicle
preparation succeeds. Unsupported insertion kinds produce an error.

## Owned objective and callback runtime

Source-qualified mode2 catalogs can describe `secure_hold` and `structural`
classes, exact indexed trigger boxes, authored timings, and structural repair
rules. `ObjectiveWorld` stages desired requests and provides a per-object update
API. A caller consumes each committed native entry before updating another
object. Initial-load catalogs remain noninteractive. Source gameplay catalogs
require the explicit concrete backend configuration described below.

SecureHold supports activation, completion, retaking, cancellation and release.
Structural demolition preserves fuse priority and defuse back to Repaired after
repair. Repair uses the objective's normalized health model, trusted actor/tool
authority and native batch timing. Entering a state is separate from sequence,
explosion, FSP and controller effects; a completed armed Structural entry marks
the required authored explosion explicitly.

`domination_callbacks::Queue` binds one immutable map and uses the supplied
network game clock. Zero-delay work is prepared immediately. Positive-delay
work retains authored insertion order, wrapping u32 deadlines and original
caller/actor identities. An update chooses the first due vector entry rather
than sorting by deadline. Failed consumers retain pending work for the enclosing
world transaction. Delayed evaluation reads the later committed snapshot.

## Match completion and requeue

```sh
python3 tools/extraction/project-retail-domination-round.py --help
```

The round projector reads starting scores, major-victory threshold and end delay
from the original retail game archive, then binds them to the exact spawn bytes
and objective metadata. A registry profile can set `domination_round` to this
absolute path. Fixed-world library callers use `Engine::with_domination_round`.
Selecting a profile starts fresh score state; replaying the same formation keeps
its current state. Switching profiles clears the previous round.

The shared native score-rate and result lifecycle publishes initial scores,
point changes, terminal notice, settlement and cohort release/requeue. Focused
Engine checks exercise opposite depletion outcomes, record-failure rollback and
fresh-round selection. The score/result setup alone cannot supply objective
entry effects or enable an interactive gameplay catalog.

## Concrete gameplay configuration

The normal Engine objective tick and authenticated interaction/repair handlers
use `domination_gameplay_provider::World`. The backend stages class entry, FSP
activity, controller callbacks, squad assignments and sequence effects together.
It prepares damage/death callbacks and bounded replication packets before
recording the transaction, then installs the candidate world. Failed preparation
or recording leaves the previous world intact. Constructor state, current state,
desired state and MissionStart requests remain distinct. Player START/RELEASE
entries are synchronous; Lua requests are evaluated by the subsequent Sim pass.

Each registry row can opt in using an absolute `domination_gameplay` path,
alongside `domination_round`, `ground_vehicles`, `character_damage_rules`, a
nonempty `collision_world`, and all three hospital inputs: `bunker_placement`,
`bunker_structure` and `bunker_allocation`. Fixed-world library callers install
the same configuration through `Engine::with_domination_gameplay`. It is off by
default. The configuration has format `mag_domination_gameplay_configuration_v1`,
schema version 1, and these explicit source paths:

- `controller`, `mission_start`, `sim_schedule` and `forward_spawns` bind the
  ordered native update and authored callback graph.
- `structural_effects` and `sequence_assets` bind both Structural and SecureHold
  sequence collision, explosions and area damage.
- `initial_assignments` binds initial manual-tag and FRAGO state.
- `core` binds aircraft-link qualification, hospital hulks, initial collision
  filters, qualified original factory/trigger poses, ground damage policy and
  its SHA-256, ground lifecycle rules and their SHA-256, and terrain material
  grid index/pixels. The hospital collision receipt includes operational bodies
  as well as destroyed hulks.

Nested backend paths resolve relative to the configuration file. Top-level
registry paths remain absolute. Sources are checked before the selected spawn
catalog is promoted or any world is installed.

The bounded `core.scope` named `core_without_helo_insertion` retains native
Helo defaults for controller decisions and uses the authored category-zero
fallback for human entry while aircraft insertion is unavailable. It does not
claim the original helicopter is absent. The worker requires `--dynamic-bodies`
and a `--static-partition` for every enabled map; the latter must prove that
queries on layers 6 and 21 exclude the immutable static roots. Resource hashes,
map identities, class coverage and all 32 squad bindings must agree.

The included projectors accept explicit original inputs and qualified external
proof; they do not acquire that proof automatically:

```sh
python3 tools/extraction/project-retail-domination-gameplay-objectives.py --help
python3 tools/extraction/project-retail-domination-structural-effects.py --help
python3 tools/extraction/project-retail-domination-ground.py --help
python3 tools/extraction/project-retail-domination-ground-lifecycle.py --help
python3 tools/extraction/project-retail-domination-hospitals.py --help
python3 tools/extraction/project-retail-domination-lalo.py --help
python3 tools/extraction/project-retail-domination-helo.py --help
python3 tools/extraction/project-retail-domination-helo-damage.py --help
python3 tools/extraction/project-retail-domination-terrain-materials.py --help
python3 tools/extraction/project-retail-character-damage-rules.py --help
```

Sequence collision uses qualified original placements and materials, including
terrain material sampling. Structural explosions and periodic area damage use
the shared authoritative character armor/death pipeline. Hospital destruction
updates the indexed body, processes its occupants and installs its collision
hulk; repair restores that body and removes the hulk. Each hospital has separate
ownership and passenger state. Boarding uses the qualified model passenger pose;
disembark requires a clear, quantized candidate against static, dynamic and
sequence/hulk collision. Exit clearance is an explicit conservative server
policy. Direct hospital hits and critical decay use the qualified vehicle damage
rules and recorded lifecycle; legacy Alpha damage cannot mutate these bodies.
Hospital gunner-seat control remains unsupported.

The optional aircraft scope is `core_with_helo_insertion`. It requires a complete
`core.helo` object containing `fleet`, `models`, `paths`, `damage` and an explicit
operator `random_seed`. Its fleet binds the exact spawn bytes and three original
source receipts; the backend checks those receipt hashes, four spawn bindings,
collision meshes and typed damage rules together. A missing pairing fails
startup, and the scope without aircraft rejects aircraft inputs rather than
silently ignoring them. The random seed is operator policy, not extracted game
data. Healthy enabled helicopters remain at the insertion point. Manual
passenger exits use the authenticated native dismount route and checked grounded
positions. FSP disable and critical retreat follow their distinct source state
and timer gates. Damage, occupant death, destruction and later reinforcement
consume the shared vehicle and character pipelines. The bounded squad policy
permits eight humans per aircraft even where the original model has more seats.

## Verification and remaining acceptance

Synthetic tests cover state/callback order, packet bounds, independent hospital
seats and rollback. Private source-data checks exercise all six map/faction
profiles through normal Engine capture, demolition and repair. They also follow
the authored refinery graph to an attacker win, passive score depletion to a
defender win, result/publication drain, cohort release and fresh two-faction
reservation. Ground checks cover normal hit/grenade input, hulk transition and
reinforcement into the same allocation. Aircraft checks cover source AA/FSP
activation, queued boarding, persistent healthy hover, manual exits, late-join
creation order, occupant death and the strict destruction-effect deadline.
Those checks use original input data and clearly identified synthetic actor or
combat inputs without launching a client.

The profiles expose qualified ground passenger entry; general ground driver and
gunner control, hospital turret control and command-ability execution remain
separate gameplay consumers. Objective progression/XP and presentation fidelity
are tracked separately from the match-completion loop.
Source checks and schema validation are not live-match acceptance. Client visual
and behavior review remains pending; releases are cut manually by the human
after that review.

## APC passenger area-effect qualification

Domination's qualified ground scope supports original physical-seat visibility through the shared [ground-vehicle projector](ground-vehicles.md#admitted-seat-visibility). Complete admitted-seat coverage is required when a bound Structural objective has enabled periodic area damage. Regenerate both ground and paired lifecycle bindings when adding visibility. Ordinary APC reinforcement and a due Structural scan share this qualified inventory; hidden passengers retain their health and mounting while the scan advances. This does not enable general driver or gunner authority.

## Playback changes with unchanged objective controls

An authored same-state request can still enter the native state body, run callbacks and select a new playback sequence. An unchanged control value therefore does not imply an empty update. The committed candidate keeps control-linked sequences in the same bounded control packet and emits other qualified playback changes through the existing sequence-only field9/field10 packets. Sequence rows retain strict index order, unique identities, valid ordinals and the normal packet-size limit. Native entry effects retain their source order.

The outer world transaction prepares every packet before recording and installing runtime, callback, sequence and world changes. A full recipient queue or recording failure defers the whole transaction. Retrying must preserve the original effect and playback semantics; dropping same-state entries to make publication succeed would lose authored behavior. Full-start source replays and native live-match acceptance remain distinct.
