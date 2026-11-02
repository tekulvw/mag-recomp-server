# Human incapacitation and resuscitation

Retail human characters can enter a downed life state after an eligible lethal
hit. They retain their existing object, pose and inventory. The server owns the
deadline and the separate finishing-damage accumulator; downed characters cannot
fire, heal themselves, mount seats or enter the respawn queue. Expiry, finishing
damage or an owned give-up request makes the character eligible for the normal
team spawn wave.

This feature is opt-in. Supply an absolute `--incapacitation-rules` path alongside
the normal retail catalogs. With the flag absent, the existing terminal-death
behavior remains available. The process's independent world scheduler advances
bleed-out without requiring patient input. An embedding must enable world ticks
and supply its monotonic clock.

## External resources

The strict rules schema has four fields. This example is **independently invented
synthetic data**, not a game preset:

```json
{
  "schema_version": 1,
  "incapacitated_damage_threshold_hp": 83.0,
  "incapacitated_duration_seconds": 7.0,
  "excluded_character_states": [22]
}
```

Initial admission compares final adjusted HP damage strictly below the threshold,
with the configured physical-state exclusions. The duration uses the effective
owned and inherited `kCharModIncapacitatedTime` modifier. Further adjusted damage
accumulates separately and finishes at or above the threshold. Expiry consumes
the downed state once and clears the two low condition bits.

The retail combat catalog also needs an explicit `incapacitated_multiplier` for
each body row. The maintained `tools/extraction/project-retail-combat.py` projects
this from the input schema's `incapMult`. It preserves the existing armor,
coverage, prevention and owned damage modifiers. Startup rejects missing body
multipliers when the feature is enabled; there is no living-body fallback.
Actual catalogs, rules, source evidence and accounts remain external private
inputs. Nothing in the public examples establishes real-client acceptance.

The Helm chart can pass this optional flag through `data.gameplayExtraArgs`, using
an operator-supplied file under `/data`. No deployment or data bundle is implied
by enabling the source feature.

## Typed requests and replication

`0x021c` is a six-byte target-only `Resuscitate` request. Its patient is a full
character handle; the healer comes from the authenticated connection's current
reservation. Both authenticated TCP and UDP plugin routes are accepted; owner
replication retains its separate UDP-only rule. The bridge parser rejects
server-originated event directions. The handler requires a living, on-foot healer, a current same-world
same-team incapacitated human patient, an unexpired deadline and a positive
effective `kCharModResuscitatePercent` modifier. Restore uses the healer's maximum
HP times that modifier, capped to the patient's maximum, then quarter quantized.

Successful restoration publishes field8 with type7 and condition2 cleared,
preserving the patient's previous hit location and attribution. It does not
publish a patient field4 override, trigger9 spawn, replacement actor or new
inventory. The patient's native controller publishes its recovery state through
the ordinary owner stream. Shared queues retain per-recipient reliable sequence
ordering. During incapacitation only owner death-state/camera presentation is
relayed; position, equipment and projectiles do not gain downed authority.
In-flight death-state reports are suppressed after revival until the owner
publishes recovery.

A typed `0x01ff` notice uses the actual healer/patient IDs, source type11,
source index−1 and false head/vehicle flags. It reaches the union of their actual
squads once per recipient. Other squads do not receive a fabricated team-wide
revive notification.

## Accounting and current boundary

The initial lethal hit owns the ordinary death/kill statistics and supported
ordinary death notification, including when it admits incapacitation. Finishing
and expiry do not replay those events. Enemy finishing and successful revival
record their distinct statistics at gameplay commit. XP acknowledgements and
repeated requests cannot increment them again. Another collapse after revival is
a new death event despite retaining the actor's physical life generation.

Optional `table.Awards.FinishingMoves` and `table.Awards.Resuscitations` configure
the corresponding action XP in the Horizon progression catalog. No amounts are
embedded in source. If an optional action award is absent, the recognized event
settles durably with no XP so it cannot poison the reward outbox. Unknown reward
kinds remain invalid.

This implementation admits **human patients**, including humans injured by
server-owned bot weapons. Server-owned bots remain terminal-only patients until
their authoritative recovery publication and AI transitions are implemented.
Mounted human death, server-side revive targeting/range/LOS and charge/cooldown
policy remain separate work; this handler does not invent them from the request.
Synthetic tests establish transaction, routing and lifecycle behavior. A bounded
native two-human revive/bleed-out test remains the client-acceptance follow-up.
