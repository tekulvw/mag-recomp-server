# Human incapacitation and resuscitation

Retail human characters can enter a downed life state after an eligible lethal
hit. They retain their existing object, pose and inventory. The server owns the
deadline and the separate finishing-damage accumulator; downed characters cannot
fire, heal themselves or mount seats. Expiry, finishing damage, an owned give-up
request (`KillSelf`), the client's bleed-out report or a respawn confirmation
ends the downed life and makes the character eligible for the normal team spawn
wave.

This feature is opt-in. Supply an absolute `--incapacitation-rules` path alongside
the normal retail catalogs. With the flag absent, the existing terminal-death
behavior remains available. The process's independent world scheduler advances
bleed-out without requiring patient input. An embedding must enable world ticks
and supply its monotonic clock.

## External resources

The strict rules schema has four required fields and an optional, audit-only
`provenance` object. This example is **independently invented synthetic data**,
not a game preset:

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

### Producing the inputs

`tools/extraction/extract-retail-incapacitation-rules.py --game-archive GAME
--output RULES` reads the threshold and bleed-out time from the character
gameplay settings in the hash-checked retail archive and records their member
digest and JSON pointers in `provenance`. That resource names no physical-state
exclusions, so the tool writes an empty `excluded_character_states`; add states
only from separately qualified evidence.

A combat catalog projected before body rows carried the field fails startup
with `incapacitation requires explicit body multipliers`. Instead of a full
reprojection, `tools/extraction/project-retail-combat-incapacitation.py --combat
COMBAT --item-catalog ITEMS --output NEW_COMBAT` adds each row's authored
`incapMult`. It requires the item catalog the combat catalog names in
`provenance.catalog_sha256`, checks that every row's source pointer, living
multiplier and armor area still match that authored entry, and keeps every other
value. Both tools write a new file and support `--check`. Validate the result
with `mag-gameplay --validate-catalogs ... --incapacitation-rules RULES` before
packaging a new data bundle.

The Helm chart can pass this optional flag through `data.gameplayExtraArgs`, using
an operator-supplied file under `/data`. No deployment or data bundle is implied
by enabling the source feature.

## Bleeding out early

A downed player can give up instead of waiting for a medic. The retail client
sends no message for the Bleed out press. It moves its death state into its
bleeding-out substate, which its next owner update reports as the downed
character state (7) with 3 in the first state-userdata byte, and shortens its
own remaining downed time to the authored `fRespawnTime` (2.5 s in the retail
character gameplay settings). When that runs out it opens the respawn screen and,
with `bRespawnWaitsForButtonPress` false, confirms its respawn by itself.

The server gives the downed life up on the first of these it sees:

- the bleeding-out report in an owner update (retail
  `SimCharacter::UpdateState` clears the incapacitated condition on it), or
- a respawn confirmation (`0x0211`) while still downed (retail
  `SimCharacter::RecvMessageConfirmRespawn` clears the condition before its
  dead check), which also joins the next wave in the same step.

The give-up publishes the same body as an expiry: the downed hit's attribution
with the downed condition bits cleared. It adds no death notice or statistics,
and the character can no longer be revived. The world tick then queues it for
the next team wave, as for any death (see [respawn-queue.md](respawn-queue.md)).
Other substates in a downed report leave the deadline alone. An owner update
whose direct reply slot is already taken defers the give-up (audited as
`downed_give_up_deferred`); the client's automatic confirmation still ends the
downed life.

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

## What lets a medic's client offer a revive

A client sends `Resuscitate` only after it decides, by itself, that its own
player may revive a downed teammate it can see. The downed body is a
revive target when its last field8 has zero health, the incapacitated condition
bit (2) set, and the body is not yet fully dead. The medic qualifies when its
kit holds a first-aid item and its own character modifiers give a positive
`kCharModResuscitatePercent`. When both hold, the medkit and the revive prompt
target the body. When the medic does not qualify, the medkit treats only living
characters below full health, which is usually the medic itself.

The client builds the modifiers for its own player once, when its Character is
created, from the owned-skill record in Character field20. That field is five
32-bit words in word order, one bit per skill (skill N is bit N mod 32 of word
N / 32). It goes from the server to the owner only. The owner's creation in the
`ClientReady` snapshot therefore carries the character's stored skill record.
Snapshots of that character sent to other players leave the field out. Without
field20, every client's own skill modifiers stay zero, and no player is ever
offered a revive, whatever skills they own. The server's own revive check uses
the same stored record through the skill catalog.

## Accounting and current boundary

The initial lethal hit owns the ordinary death/kill statistics and supported
ordinary death notification, including when it admits incapacitation. Finishing
and expiry do not replay those events. Enemy finishing and successful revival
record their distinct statistics at gameplay commit. XP acknowledgements and
repeated requests cannot increment them again. Another collapse after revival is
a new death event despite retaining the actor's physical life generation.

Hostile human bullet hits and queued grenades also stage the normal
`EnemyKilled` reward at initial collapse. Credit belongs to the actual
same-world authenticated source, including its already-queued blast after death.
Friendly fire, self damage and attribution naming another actor do not grant it.
The receipt combines the actual victim identity and physical generation with
the staged next retained death count. A second collapse after same-object revival
therefore has a distinct identity. Invalid or exhausted ordinals suppress the
ambiguous reward with an audit diagnostic while damage and statistics continue.
All awards still use the external progression catalog's configured amounts.

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
