# Acquisition controller planning

`acquisition_control::Catalog` loads a bounded external graph tied to the exact
spawn catalog SHA256 and selected Acquisition map. Its four chains identify each
capture objective, Arrival objective, escort-vehicle spawn point and volume.
These are authored tokens, not native objective pool indices. The graph must
cover two stages in each of two platoons, with distinct identities and valid
vehicle-spawn joins. All conditional/action targets must belong to their declared
objective or volume family. No map tokens or original script are included here.

The catalog contains four ordered conditional groups. A group selects its first
matching branch; all groups contribute actions in source order. The public Rust
implementation is a generic bounded planner, with the branches and targets in
external configuration. `plan` requires observations of all eight objectives as
`completed`, `deactivated` or `other`. Missing observations fail instead of being
treated as another state. The input snapshot remains unchanged, and evaluating
it again produces the same intents.

Commands express objective lock/enable/disable calls and damage-volume calls
with explicit four integer arguments and symbolic damage/source types. Neither
damage amounts nor enabled/disabled volume semantics are inferred. Native
enable/disable/lock calls request target states without immediately changing
current-state predicates. Applying intents still requires qualified token-to-ID
bindings, the native target guards, and the existing record-before-commit path.
This increment does not attach the planner to gameplay callbacks or emit packets.

`Plan::unbound_calls` retains unresolved trailing calls separately. Such calls
are never executed by this planner. A consumer must not treat a nonempty list as
a complete implementation of the original script's effects. Callback selection
and scheduling, authoritative arrival/destruction, objective replication, score
progression and match-end rules remain separate work.

## Projection and offline checks

`tools/extraction/project-retail-acquisition-control.py` accepts:

```sh
python3 tools/extraction/project-retail-acquisition-control.py \
  --spawn-catalog "$MAG_SPAWNS" --graph-proof "$MAG_CONTROL_PROOF" \
  --game-archive "$MAG_RETAIL_GAME" --output "$MAG_CONTROLLER" --check
```

It verifies the original archive fingerprint, reads the script member named by
the supplied evidence, checks source hashes and parameter/chain/controller joins,
and parses a restricted grammar without executing Lua. Condition groups and
their command order come from that external script. Comments and literal debug
prints do not become actions; unsupported statements, nested control flow,
unknown conditional calls or incomplete functions fail. Integer-only trailing
calls remain explicitly unbound. Original cross-chain targets are preserved.
Proof hashes and structural joins do not independently certify native semantics.

The standalone Rust consumer loads and evaluates the actual catalogs without
starting an engine, listener or account store:

```sh
cargo run --manifest-path gameplay/Cargo.toml \
  --example validate_acquisition_control -- \
  "$MAG_SPAWNS" "$MAG_CONTROLLER" "$MAG_OBSERVATIONS"
```

Observations are a JSON object mapping decimal authored token strings to the
three classifications. Synthetic Rust/Python checks cover branch priority,
multiple-group ordering, complete observations, source identity, target-family
validation, external volume targets and parser rejection. Original-derived
catalogs and native analysis remain private. Live-match review and manual
release remain separate from checked source integration.
