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

## Connected gameplay work

The next backend must perform class-specific state entry and linked FSP effects
before the callback, and squad assignment refresh afterward. Native startup
requests, constructor state, current state and desired state need distinct
representations. Actor attribution, source order, replay and delayed calls must
survive record-before-install transactions. The backend must connect capture,
repair/demolition, bunker/vehicle lifecycle and actual placement to those
commands before enabling the controller in a live worker.

The common score/result publisher can carry the resulting point changes,
terminal notice, settlement and cohort release. Runtime callback epochs,
source-specific match-end rules and fresh-round reset remain separate checked
work. Native gameplay acceptance and release decisions follow live-match review.
