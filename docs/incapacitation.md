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
with the configured physical-state exclusions. A lethal hit whose damage type carries
KineticResponse (bit 8 of the native type, which explosives such as frags and
rockets set) is never admitted: the client marks such a death as complete as
soon as it applies that field8, and goes straight to the respawn screen without
a down. Hits without the flag, such as bullets, can still down a player. The duration uses the effective
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
the first team wave past the respawn queue's 3.5 s cutoff, as for any death,
so the new life arrives only after the client's respawn screen has opened
`fRespawnTime` (2.5 s) after the give-up (see [respawn-queue.md](respawn-queue.md)).
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

Sending field20 is opt-in with the worker flag `--owner-skills-record`; it is
off by default. In a lab run on gameplay 0.17.0, which sent it unconditionally,
a downed client with an empty skill record showed the incapacitated HUD and
then went to the respawn screen about 2.5 s later, as if its downed time were
zero. Until a lab A/B shows the flag leaves the client's downed timer intact,
keep it off. With it off, the owner's creation is the same as before 0.17.0,
and clients are again not offered revives.

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

This implementation admits human patients, including humans injured by
server-owned bot weapons, and world-owned bots (below). Mounted human death, server-side revive targeting/range/LOS and charge/cooldown
policy remain separate work; this handler does not invent them from the request.
Synthetic tests establish transaction, routing and lifecycle behavior. A bounded
native two-human revive/bleed-out test remains the client-acceptance follow-up.

## Downed bots

World-owned bots go down under the same rules as humans when
`--incapacitation-rules` is supplied: an eligible lethal hit below the
threshold downs them, and a lethal KineticResponse hit (grenades, rockets)
is final. Bots own no skills, so their downed time is the authored one. The
bot keeps its body and life, publishes the same field8 as a downed human (zero
health, condition bit 2) with the death state 7, and late joiners receive that
state in its snapshot. While downed it cannot move, fire, heal, take a passage
or be targeted, and it does not join a respawn wave.

- **Finishing:** later hits add to the separate finishing budget with the
  incapacitated body multipliers. Reaching the threshold finishes the bot; a
  human finisher on the other side records `EnemyFinished` and statistic 24.
  As for humans, the kill, the kill-feed notice, `EnemyKilled` and the bot's
  firearm K/D belong to the hit that downed it; finishing adds none of them.
  Bots never aim at a body at zero health, so they do not finish downed
  characters.
- **Human revive:** `Resuscitate` naming a downed bot on the healer's side is
  accepted under the human conditions (living on-foot healer, unexpired
  downed time, positive `kCharModResuscitatePercent`). The restore is the
  healer's maximum health times that modifier, capped at the bot's maximum.
  Both squads hear the notice, and the healer records statistic 34 and the
  `TeammateResuscitated` XP, except when the downing hit came from the
  healer's own side. The server owns the bot, so it publishes the recovery
  itself: the restored field8 with the ordinary standing state. The bot keeps
  its life and position and resumes its AI from there. A second collapse in
  that life is a new kill.
- **Bot revive:** the nearest free bot medic within `medic_radius_m` walks
  to a downed teammate, bot or human, and revives it with its medkit, as it
  revives a downed human (see "Reaching downed teammates" in
  [bot-healing.md](bot-healing.md)).
- **Bleed-out:** the world clock ends a downed bot life at the authored
  deadline. Bots never press Bleed out, so a bot also gives up when no medic
  can help: `medic_check_delay_ms` (10 s) after it went down it looks for a
  friendly medic within `medic_radius_m` (30 m, straight-line). A medic is a
  living, on-foot, not downed character on its side that can revive: a human
  carrying the medkit with a positive `kCharModResuscitatePercent`, or a bot
  carrying the medkit while bot revives are on. With none, the bot bleeds out
  at once. With one, it checks again every `medic_recheck_ms` (1 s) and bleeds
  out at the first check that finds none, never later than the authored
  deadline. A bleed-out publishes the downing attribution with the downed
  condition bits cleared, as a human give-up does, and the bot joins the first
  team wave after the bleed-out (`bot_bled_out`, with the reason
  `no_medic_in_range` or `expired`). Bots have no client still playing a
  death, so the respawn queue cutoff (see [respawn-queue.md](respawn-queue.md))
  does not apply to them.

The population file's `incapacitation` object configures this; omitted fields
use the defaults:

```json
{
  "enabled": true,
  "medic_check_delay_ms": 10000,
  "medic_recheck_ms": 1000,
  "medic_radius_m": 30.0
}
```

`"enabled": false` keeps every bot death terminal. Bot falls stay terminal.
Whether the original client offers its revive prompt on a server-owned body
has not been observed yet; it needs a lab check with a human medic who owns
the resuscitation skill.
