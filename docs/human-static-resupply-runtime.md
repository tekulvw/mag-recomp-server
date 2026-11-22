# Human static grenade resupply

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
current living, entered, initialized on-foot Character, a selected ordinary frag
and the current accepted position inside the authored oriented box. There is no
automatic overlap refill. Stop addresses one exact provider session. Repeated
Start preserves that session's interval and progress.

The server world scheduler tracks elapsed time per recipient and provider.
A new interval uses the authored tick rate and the effective
`kCharModResupplyRate` modifier. On a due update the worker consumes one stored
interval, refreshes it from the current modifier, and fills the selected frag
budget to its authored cap. The normal grenade launch path spends that budget.
The worker requires a finite, positive effective interval. This defensive
bound keeps invalid external tuning from scheduling a continuous refill.
The worker records a pulse before changing the session or ammunition. It emits
no new ammo-count message. The retail client has separate local clip and reserve
timing.

The replacement session is bound to the current reservation, actor life and
round. It ends on a mounted or dead actor, a departed or unavailable provider,
or a stale round. The due-time range check is a replacement policy. Original
static Start checks the box while the observed original due path resolves the
recipient without repeating that geometry test. The worker does not use the
bot's dry-ammo, idle or perception rules. The grenade feature leaves health and
other equipment untouched.
