# Human static supply healing and grenade resupply

Set `human_static_resupply` in the Acquisition gameplay configuration, or
`core.human_static_resupply` in the Domination gameplay configuration, to the
owner-projected catalog for the selected map and attacking faction. Paths
resolve relative to the gameplay configuration file. The fields are accepted
through direct Acquisition configuration and registered match profiles.
The supported Acquisition maps are Alaska Subpen, Panama and VOZ. The
supported Domination maps are Baku, Canal and Unalaska.
Omitting the field disables human static resupply. Use offline
`--validate-catalogs` startup to verify the resource binding. The readiness output
reports `human_static_resupply_validated`.

The worker uses the supplied catalog's exact family-42 target IDs and requires
the matching map, mode, attacking faction, source ELF, spawn bytes, collision
world bytes, and retail combat catalog. Its source provenance comes from the
owner's validated extraction and the fresh successful compiled allocation model.
The worker consumes a neutral catalog and does not read original ELF or level
archive bytes. Catalog readiness provides no live client coverage.

Start and Stop use the authenticated retail `0x020b` route. Start requires a
current living, entered, initialized on-foot Character at an accepted position
inside the authored oriented box. A provider with a positive health percentage
can serve a Character without a selected frag. There is no automatic overlap
refill. Stop addresses one exact provider session. Repeated
Start preserves that session's interval and progress.

The server world scheduler tracks elapsed time per recipient and provider.
A new interval uses the authored tick rate and the effective
`kCharModResupplyRate` modifier. On a due update the worker consumes one stored
interval and refreshes it from the current modifier. The worker requires a
finite, positive effective interval. If an ordinary frag is selected, the pulse
fills its launch budget to the authored cap. The retail client uses separate
local clip and reserve timing.

A positive authored percentage restores that fraction of effective maximum HP
using single-precision arithmetic, capped at missing HP. Each application
truncates the resulting health to quarter-HP precision. The ordinary type-6
Character health update retains previous hit attribution. Supply healing is
uncredited and awards no XP. The worker publishes one ordered health update
per healed actor per tick. It records the session, ammo, health and updates
together before installing them. If the health packets cannot fit in the
reliable queue, the entire candidate stays due and the scheduler continues
draining the queue.

The replacement session is bound to the current reservation, actor life and
round. It ends on a mounted or dead actor, a departed or unavailable provider,
or a stale round. The due-time range check is a replacement policy. Original
static Start checks the box while the observed original due path resolves the
recipient without repeating that geometry test. The worker does not use the
bot's dry-ammo, idle or perception rules. Other equipment stays unchanged.
