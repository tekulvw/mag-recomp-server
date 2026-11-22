# Rocket death notifications

The optional retail stock infantry, guided and underbarrel launcher paths queue
the ordinary death notification when an admitted on-foot Character first reaches
zero health. The shared Hospital secondary Character backend uses this same
source-qualified path. Existing damage, incapacitation, kill and assist credit
rules remain responsible for those effects.

The notification uses Weapon source category 2 and the sign-extended 16-bit
equipment index retained at launch. Underbarrel projectiles retain the launcher
attachment's index. The packed projectile remains the damage source object;
it is independent of the notification's equipment index. Equipment changes
after firing cannot change the notice. A shooter who dies or respawns within
the same authenticated allocation retains attribution; a replaced allocation
cannot inherit the flight.

The source binding follows the retail `0210` receiver's selected kit row
`globalEqIdx` at `7dcad0/7dcae0`, its creation argument and projectile `+0x74`,
then the common Character Debris callback at `108b97c..108b9d8`. The original
first-collapse callback forwards Weapon2 and that signed index to `01ff` before
the separate incapacitation choice. The ordinary blast's location 1 maps to a
non-head body region.

Health/life changes, consequences and pending notices are staged together and
installed after the terminal record succeeds. A full notification FIFO defers
the complete terminal. Reliable health frames drain before the application
notice, and repeated terminal work cannot repeat the first-collapse notice.
Actual attacker and victim squads select recipients, with same-squad recipients
deduplicated by the existing notice path.

Synthetic regressions exercise authenticated stock, guided/dumb and
stock/upgraded underbarrel launches, retained equipment, posthumous attribution,
stale allocations, record refusal, queue pressure and health-before-notice
ordering. Original-source offline replay and native client presentation are
separate evidence. This increment adds no mounted-victim or finishing/bleed-out
policy, and releases remain the maintainer's manual action after live review.
