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

The generator preserves every spawn record and type. For attacker squads with
an unavailable APC insertion, an authored Basic alternate supplies the explicit
entry fallback. Other special insertion kinds remain unavailable in this
foundation. The inactive default objective state and no-assignment sentinel
are retained until a separately qualified MissionStart transaction.

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
object. Initial-load catalogs remain noninteractive. Engine admission refuses
source gameplay catalogs until a concrete Domination backend is available.

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

## Remaining connected gameplay

The backend must complete class-specific entry, authored gameplay sequences and
explosions, linked vehicle hooks, callbacks and native squad assignment refresh
inside the recorded world transaction. Startup requests, constructor/current
state and desired state remain distinct. Hospital identities, per-profile
bunker ownership and special insertion consumers must be qualified before those
points become selectable. Source checks are not a live-match acceptance claim.
Native gameplay acceptance and manual release decisions follow live-match review.
