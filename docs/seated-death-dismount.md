# Seated death leaves the seat

A character that dies or goes down while it occupies a seat leaves the seat
in the update that publishes the death. This holds for every seat family the
server runs (indexed Hospital and legacy Alpha bunkers, Ground vehicles in any
seat, and Helo passengers) and for every cause of death (KillSelf, firearm
hits, grenade and other pending damage, blast, burning, volume and vehicle or
structure destruction effects).

## Behaviour

The original server treats a death in a seat as a seat exit. When a mounted
character's state changes away from the mounted state, the vehicle releases
that occupant. The occupant is then given the downed state (7) when it is not
alive, or the on-foot state (1) otherwise, together with a server-chosen exit
position and facing. Ownership of the vehicle changes in the same frame.

A client that is still seated does not act on a downed state alone. In a lab
run of the replacement server, a seated KillSelf that published only the
downed state and terminal health left the client seated. It then asked to
dismount once a second, and each request was refused because the seat had
already been released. The death showed later, when the player next left the
seat. Both sides kept the wrong state for the rest of the session: deaths were
counted that the client never showed, and a later seated spawn put the player
on foot with no reserve ammunition.

## Replacement rule

1. One resolver, `Engine::seated_death_exit`, maps an occupant's current seat
   to an exit:
   - Hospital: the current clear authored exit, or else the bunker's authored
     preferred exit with no camera preference.
   - Legacy Alpha bunker: the configured exit with no camera preference.
   - Ground: the current qualified vehicle exit, or else the vehicle's authored
     preferred exit.
   - Helo: the carrier's current body position and heading.
   - When none of these resolves, the occupant's last tracked position and
     facing.
2. Every lethal or downing Character damage update for a human goes through
   `Engine::character_damage_update`. If the target is seated, the update's
   Character record carries state 7, the exit position (field 5), the exit
   facing (field 7) and the damage (field 8). A bunker exit also carries the
   bunker's post-release ownership in the same packet. A Ground driver's exit
   also returns driving authority to the server in the same packet.
3. Releasing a seat on death always goes through `Active::release_seat_on_death`.
   It clears every seat mirror and remembers which seat was left. A dead
   character asking to dismount from that seat gets a freshly sequenced copy of
   the terminal exit instead of a refusal. The record is cleared by the next
   life.
4. Other effects keep their existing paths: turret-grant revocation, the
   Hospital and Ground ownership corrections already sent to every recipient,
   and Helo fleet release on KillSelf. These follow the terminal update in the
   same event.

Bots are not seated in this server and keep their existing death updates.

## Checks

Synthetic native-event tests cover a seated death for each family and cause
that can reach a seated human here. Each death publishes one update with
state 7, the exit position and facing, and the damage. A dead occupant's
repeated dismount gets the terminal exit instead of `bunker_seat_not_owned`.
A Hospital death followed by the next wave seats the new life with its full
loadout. Live controls, camera behaviour and presentation still need a human
lab review.
