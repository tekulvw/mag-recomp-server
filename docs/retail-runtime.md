# Explicit retail gameplay startup

Select application `20231` explicitly when starting the retail gameplay bridge.
The selected application becomes the initial wire profile and is fixed for the
life of that runtime instance. A later `ConfigureMatch` request for another
application fails. The legacy mode without `--application-id` remains available
for existing prototype integrations; it is not the retail startup contract.

Retail startup requires all nine inputs below as explicit absolute paths. There
are no prototype sibling-file defaults for these inputs when retail is selected.
Catalogs stay outside the public source tree and container image.

| Gameplay argument | External input |
| --- | --- |
| `--item-catalog` | Raw retail item catalog, schema 2/application 20231 |
| `--spawn-catalog` | Selected neutral runtime spawn catalog and explicit match feature policy |
| `--wire-profiles` | Wire profile catalog containing application 20231; a retail-only catalog is accepted |
| `--character-blast-rules` | Qualified shared character blast rules accepted for the selected application |
| `--bootstrap-loadouts` | Retail starter equipment for all three supported factions |
| `--retail-combat-metadata` | Retail combat projection matching the raw item catalog |
| `--retail-armory-catalog` | Retail saved-equipment projection matching the raw item catalog |
| `--skill-catalog` | Raw retail skill table, schema 2/application 20231, for owned character modifiers |
| `--fall-damage-rules` | Retail falling-damage rules; this input is required and falling damage is enabled |

## Validate privately before starting services

Set `CATALOG_DIR` to an absolute directory containing the operator-selected
catalogs. The following filenames illustrate the input roles; choose filenames
that match the private extraction and selected map.

```sh
: "${CATALOG_DIR:?Set an absolute private catalog directory}"

mag-gameplay --application-id 20231 --validate-catalogs \
  --item-catalog "$CATALOG_DIR/retail-items.json" \
  --spawn-catalog "$CATALOG_DIR/selected-retail-spawns.json" \
  --wire-profiles "$CATALOG_DIR/wire-profiles.json" \
  --character-blast-rules "$CATALOG_DIR/character-blast-rules.json" \
  --bootstrap-loadouts "$CATALOG_DIR/retail-all-factions-bootstrap-loadouts.json" \
  --retail-combat-metadata "$CATALOG_DIR/retail-combat.json" \
  --retail-armory-catalog "$CATALOG_DIR/retail-armory.json" \
  --skill-catalog "$CATALOG_DIR/retail-skills.json" \
  --fall-damage-rules "$CATALOG_DIR/retail-falling-damage.json"
```

`--validate-catalogs` loads and validates the selected catalogs, then exits. It
requires `--application-id`, opens no gameplay listener, starts no world
scheduler, and does not create gameplay or telemetry logs. Omit `--log` and
`--tick-telemetry`; supplying either is an error. A successful receipt has
`event: "catalogs_validated"` and identifies its scope as schema and profile
validation, without claiming client acceptance.

## Event log rotation and retention

Gameplay records every accepted action in its `--log` evidence file and refuses
actions it cannot record. Without rotation that file is capped at 64 MiB; once
it is full, gameplay rejects every client event and Medius ends the affected
sessions. `--log-segment-bytes BYTES` (at least 65536) removes that cap: the log,
and `--tick-telemetry` when enabled, continue in numbered segments
(`events.jsonl.000001`, ...), rotating only between whole records. Every segment
is created new; existing files are never overwritten. `--log-retention-s
SECONDS` additionally deletes closed segments once they have been closed that
long; the active segment is never deleted. Sequence numbers continue across
segments, so retention gaps are visible. Retention bounds disk use by age, not
size: choose it so the expected log rate over that period fits the log volume.
Both flags are rejected with `--validate-catalogs` and `--collision-query`.

A served worker writes the event log from its own thread, so a slow disk never
holds the world lock that ticks and bridge requests share. Up to 32 MiB of
records wait in memory; beyond that, records are dropped rather than stalling
the match, each drop leaves a gap in the record sequence numbers, and stderr
reports `{"event":"event_log_records_dropped","dropped":N,"dropped_total":T}`
once the writer catches up. A storage error still stops gameplay from accepting
actions, one record later than a direct write would.

The bridge tolerates short worker stalls: event requests wait up to 6 seconds
and control requests 2 seconds, and Medius ends the world stream only after 12
seconds without a frame (gameplay sends a heartbeat every second and ends a
subscription after 10 silent seconds). A recipient whose reliable shared or
notification queue exceeds 512 entries is evicted with
`replication_backlog_released`, which records both queue lengths.

### Resuming the world stream (`world_stream_resume_v1`)

A served worker numbers each world frame it sends within an epoch drawn once per
process, and keeps the last 512 frames (at most about 16 MiB) in a journal.
Medius asks for these positions when the worker's hello lists
`world_stream_resume_v1`. The acknowledgement of a subscription carries the
position the next frame follows, and Medius checks that every frame is exactly
the next one. When the stream breaks with a transport error, Medius keeps every
reservation (players stay connected and their events still reach the worker,
while admission stays closed), reconfigures a legacy match, and subscribes again
with `resume: {epoch, after}` naming the last frame it routed. The same process
replays the frames after that position and continues live, and Medius logs
`Gameplay world stream resumed`. Medius releases the players with
`world_stream_gap`, as before, when:

- the worker refuses the resume with `world_resume_unavailable`, because it is
  another process (a restart) or no longer holds the frames;
- 20 seconds pass without a resumed stream;
- a frame is out of sequence, misaddressed or malformed, or the worker reports
  `match_not_configured`.

Workers and Medius without the capability keep the previous behaviour, and a
configured worker registry never resumes. The worker produces frames only while
a subscription is registered, so the journal only has to cover frames in flight
when a stream broke.

### Retrying events (`event_replay_v1`)

A served worker keeps its last 512 event replies by request ID and answers a
repeated ID with the original reply without applying the event again. When the
worker's hello lists `event_replay_v1`, Medius sends an unanswered event again
with the same request ID, up to three attempts in all, 250 ms apart, before any
later event in the same queue. Without the capability Medius never repeats an
event, because a repeat could apply an uncertain shot twice.

Do not combine the validator with `--collision-query`; diagnostic queries are a
separate command. When an explicit application selects an optional collision
world, its map must match the selected spawn catalog in both offline validation
and normal startup.

Validation checks the selected item and skill applications, the wire profile,
starter equipment availability for all factions, and starter equipment validity
against the retail Armory. Bootstrap, combat, Armory and raw skill metadata must
identify the SHA-256 of the exact selected raw item JSON bytes. Changing those
bytes requires matching projections; a shared application ID alone is
insufficient. These links check consistency between supplied files. They do not
independently prove that the files were extracted from the intended originals.

Every populated starter and saved-equipment slot must also resolve through the
raw retail item catalog. Armory membership alone cannot accept an unresolved raw
item definition. Admission and equipment replication enforce this check across
all five inventories, including inventories that are not currently selected.
Unresolved item IDs fail explicitly; they are not renamed or converted.

The neutral spawn schema does not encode an application identity. Operators must
verify its source provenance, selected map, match policy and runtime identity
evidence separately. A successful application/profile check cannot establish
that a neutral spawn file came from retail. Existing retail entry catalogs can
retain unverified pool indices and pre-terrain-snap positions; parsing them does
not upgrade that evidence.

When ready for an authorized service start, use the same explicit inputs, remove
`--validate-catalogs`, and add `--log` pointing to a new runtime log outside the
catalog directory. The executable refuses to overwrite an existing log. Choose
the bind address and any additional optional feature inputs through the private
deployment configuration. This document does not change chart interfaces or
version pins.

## Enabled features and remaining limits

Retail bunker authority is unsupported in this startup mode. The selected match
profile must explicitly disable bunkers. Do not supply `--structure-catalog`,
`--bunker-catalog` or `--bunker-exit-policy`; these inputs are rejected for retail.
The runtime carries no bunker world and reports the feature as disabled because
its authority is unverified. It still publishes spawn-pool setup. This is not a
claim that the original map contains no bunkers.

Objectives require a compatible explicit objective catalog whenever the selected
match policy enables them. The offline validator checks the objective map and
wire profile before returning success. An entry policy that disables objectives
does not establish complete original match behavior.

Falling damage uses the required retail rules. Other combat behavior remains
bounded by the authority declarations and implemented paths in the supplied
combat catalog. Candidate ammo tokens do not become a generally verified live
ammo table, and unresolved authored item definitions remain unsupported. The
runtime and validator make no claim that all retail items, protocol operations or gameplay features are resolved. Private
client acceptance remains separate from public synthetic checks.

## Gameplay skills and Medius skill purchases are different inputs

Gameplay's `--skill-catalog` consumes the **raw retail skill table**, commonly
named `retail-skills.json`. It resolves the modifiers for a character's already
owned skill bits.

Medius uses `MAG_RETAIL_SKILL_CATALOG` for the **retail skill purchase projection**,
commonly named `retail-skill-purchases.json`. That projection includes purchase
tiers, costs and qualified equipment-award bindings. Supplying the raw modifier
table there is insufficient. Prepare it with the separate
[retail skill purchase projector](native-projectors.md#retail-skill-purchases).
Medius selects application 20231 through its `MagApplicationId` configuration and
uses `MAG_RETAIL_ARMORY_CATALOG` for retail saved equipment. Prototype
`MAG_SKILL_CATALOG` and `MAG_LOADOUT_DEFAULTS` inputs do not substitute for those
retail projections.

Use the private bundle workflow in [data-bundles.md](data-bundles.md) to select
and distribute the exact inputs needed by each service. The bundle tool does
not infer these distinctions from filenames.

## Objective-driven forward and parachute spawns

The optional `maps[].forward_spawns` array links an objective's completion to a
squad's available points and shared default. Each entry contains
`objective_index`, `squad_index` and `spawn_point_index`. These are explicit
external runtime indices. They require `runtime_indices_verified: true` and a
nonempty `runtime_indices_basis`, describing initial-load pool order. This
identity qualification does not certify live behavior at every point.

Bindings must reference existing squads and initially inactive supported points,
exclude those points from initial availability, and cannot share a target across
incompatible squad sides/factions. Duplicate objective/squad and squad/point
bindings fail validation. The selected objective catalog must contain each
referenced objective and squad. Both startup and `--validate-catalogs` check these
links before opening listeners. An objective's owner side can describe the
opposing structure being completed; it does not imply that the benefiting squad
has that same side. Catalogs without forward bindings retain their existing
initial availability and selection behavior.

Only the bound objective reaching `Completed` (native state 7) activates a rule.
Assigning a later objective, or merely activating the bound objective, does not.
Completion updates every affected squad, including empty squads, and new human
admissions inherit the progressed default. Existing members following the old
shared default follow the new point; personal overrides remain selected. The
runtime publishes an availability-only GameState update, squad assignments and
private character assignments through the existing reliable queues. This does
not move or respawn a living character. Subsequent leader selections remain
constrained to the squad's currently available points.

A spawn location with `kind: "parachute"` requires an external `parachute` object:

```json
{
  "forward": [0.6, 0.0, 0.8],
  "spacing_m": 5.0,
  "yaw": 377
}
```

These example values are synthetic. `position` is the reviewed world-space
**airborne center**; the server supplies no launch height or terrain correction.
`forward` must be finite and approximately unit length (squared-length tolerance
0.001); spacing must be finite, positive and at most 64 meters; yaw is the native
10-bit value 0–1023. All eight possible offsets must fit the existing position
wire range. Other spawn kinds cannot carry a parachute pose, and parachute points
must begin inactive and unavailable.

A committed human parachute respawn carries character state 17, stance 0, zero user
data, the external yaw, normal inventory fields and no mounted vehicle state.
It requires a resolved replication inventory. Ordinary on-foot spawns retain
state 1. For each point and actual pending respawn-wave deadline, committed humans
use ordinal 0, 1, … and `center + forward * spacing_m * ordinal`, bounded by configured
squad capacity. Selection, confirmation retries, deferred replies, recording
failures and replication replay do not consume an ordinal. New wave deadlines
start at 0; delayed older pending waves retain their own counter. Round reset
clears activation, shared defaults and counters using immutable in-memory source
bindings, without rereading files.

This support covers the human candidate/protocol path and offline synthetic
verification. It does not establish aircraft simulation or full original client
entry parity. Bots retain their existing physical population and respawn paths;
objective progression does not teleport them into parachute locations. Extracted
world poses, runtime identity evidence and any private client acceptance remain
external inputs and evidence, outside public source distribution.
