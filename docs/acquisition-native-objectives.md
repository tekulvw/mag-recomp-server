# Qualified initial Acquisition objectives

The retail objective catalog accepts Sabotage mode0/eight squads and Acquisition
mode1/sixteen squads. Squad addresses preserve both platoons on both sides.
Ordered source objective vectors retain duplicates and authored Order values;
Order is not a sorting key. Acquisition may explicitly use objective index65535
when no initial objective is available. It cannot use that sentinel while an
available objective exists. Sabotage's existing assignment validation is retained.

Existing `bounded_concurrent_token_slot_capture` qualification remains supported.
The separate `native_initial_load` qualification requires
`initial_load_scope: "pre_mission_start_before_dynamic_creation"`. It describes
a successful normal compiled client load into a fresh/reset entity pool **and
freshly constructed corresponding NetObjects**, before MissionStart, incoming
state, post-load or dynamic mutation. Resetting only entities does not establish
the NetObject constructor state. It does not claim live occupancy or a capture.

Legacy source field names remain compatible. `binding_capture_sha256` identifies
the token/slot evidence selected by `capture_qualification`; under the source
model it names a static native receipt. `startup_proof_sha256` identifies the
initial-state evidence and `source_image_sha256` the original native image.
Hash syntax and structural validation do not independently certify that evidence.

This initial-load path accepts only inactive constructor network states, empty
initial assignments, and no interaction, demolition, scoring, reward, progression
or match-end rules. Authored Active and saved desired state are separate facts.
They are never promoted into current network state by this projector. A distinct
qualification is required before enabling post-start gameplay effects.

## Projection

```sh
python3 tools/extraction/project-retail-acquisition-initial-objectives.py \
  --spawn-catalog "$MAG_SPAWNS" --bindings "$MAG_OBJECTIVE_BINDINGS" \
  --native-elf "$MAG_RETAIL_ELF" --output "$MAG_INITIAL_OBJECTIVES" --check
```

The projector joins original level hashes and the complete qualified inventory.
It uses only `native_initial_client_binding` for the explicit client slot,
packed entity identity, type12 network index and constructor state. Candidate
ordinals and unresolved runtime/Sim fields are ignored. Trigger volumes are
outside the qualified objective inventory. Output objects are sorted by verified
network index; squad entries retain their original vector order.

These catalogs use the existing ObjectiveCatalog/ObjectiveWorld startup,
replication and late-join paths, including ordinary sequence/replay handling.
They contain no controller event adapter. Completing native controller gameplay
requires correctly ordered callbacks and full command effects.

In particular, native DamageVolume performs an immediate temporary overlap query
and applies damage synchronously. Its numeric arguments do not configure a
persistent volume. Negative damage must not be approximated with a partial
victim list. Authoritative geometry/filtering, actor/vehicle/mine consequences,
arrival, scoring and results remain separate work. No live acceptance, service
change or release follows from offline catalog verification.
