# Retail poison grenades

Generate the external rules with the [retail gas projector](gas-grenades.md).

Opt in with `gas_grenades` in an Acquisition gameplay configuration or
`core.gas_grenades` in a Domination profile. The path is relative to the
configuration file. Use the generated `mag_retail_gas_grenades_v1` rules,
matching retail combat and Armory metadata, and qualified Character collision
bodies. The rules bind the installation's executable, archive, item identity,
bootstrap and skill-purchase provenance. Equipment admission requires the
configured faction item in the active gear-4 slot and its owned unlock.

The existing finite grenade count, launch and Timer/Volume owner message
drive the effect. Static supply refills the spent grenade count at the
provider's configured cadence. A terminal cloud publishes through the
shared family-5 pool. Live owners receive the volume row and peers also
receive the projectile state-2 transition. Late join snapshots retain both
identities until expiry. The wire has no elapsed-volume field, so a newcomer
can see a locally restarted visual.

The world clock samples the source-derived lifetime, warmup and interval.
Each cloud consumes at most one scheduled pulse per update and carries any
remaining elapsed time. A pulse uses its authored per-pulse Character dose
and radial falloff. It shares the existing poison Character receiver, its
protected-seat and armor rules, and a single current-life cooldown with
leader gas. The response does not run a material ray or affect structures,
vehicles or objectives. The pulse candidate records before installing
damage, rewards, cooldown, and publication; reliable queue pressure defers
the batch until later world work can drain it.

The cloud survives a thrower's death within the same reservation and
allocation. Disconnect or teardown uses the existing removal path for both
objects. If the source identity no longer matches at a pulse, the damage
has no credited attacker. The damage model still receives the retained
parent projectile as its source object. A full family-5 pool defers the terminal rather
than evicting a visible volume.
