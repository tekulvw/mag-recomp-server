# Rifle underbarrel launchers

The optional retail path accepts the three stock-rifle launchers projected by
`tools/extraction/project-retail-underbarrel-launchers.py`. The owner supplies
the original files and keeps the generated rules outside the public source.
Set `underbarrel_launchers` in Acquisition configuration, or
`core.underbarrel_launchers` in Domination configuration, to that file.
Relative paths resolve against the configuration. Omission leaves the feature
disabled. `--validate-catalogs` validates resources without opening a listener;
readiness reports `underbarrel_launchers_validated`.

The catalog binds the loaded retail item/combat source hashes, original native
build, exact faction rifle/attachment pairing, separate launcher WeaponData,
finite magazine and ammo definition. Normal saved-loadout eligibility remains
responsible for purchasing and equipping the attachment. Required contact and
classified damage resources are the same as the supported infantry launcher
profile. Switching profiles clears or replaces this optional binding.

## Fire and ammunition

Authenticated native `0210` requests use ammo index17 for the equipped primary
launcher. The projectile slot is a separate identity. The owner must be alive,
initialized, on foot, and in an open combat round. Unsupported rifles, factions
and attachment combinations remain refused. Ordinary rifle hits continue to
use the host rifle's ammunition and damage path.

The launcher has its own life-scoped finite supply and authored refire fence.
Firing an equipped rocket does not replenish it, and firing a grenade does not
replenish rockets. Exact active-launch retries do not spend ammunition twice.
The native client handles reload timing and presentation. An admitted static
supply pulse stages both allowances, frag replenishment and any health change
in one recorded transaction, retaining active flights and refire fences.

The native writer clamps reported launch power; the server applies the current
owned throw-power modifier to the decoded value once. The launch quaternion
rotates authored local forward speed, and the transmitted additional world
velocity is added once. As with current rockets, launch position and additional
velocity remain authenticated client spatial inputs. Independent muzzle/origin
validation is a separate boundary.

## Server motion and impact

The server advances a translated capsule with source-derived world gravity and
authored damping. Work is bounded by fixed steps, sweep counts and the shared
collision query budget. Current static, sequence and dynamic bodies supply
contacts. Capsule orientation remains fixed at launch.

Arming eligibility uses the enclosing bounded update end, consistent with
timer decrement before native contact consumption. An early contact consumes
that event and preserves the projectile. The replacement uses authored
restitution for a normal response and backs off along already clear incoming
space. An unchanged overlap cannot become a delayed armed impact. An unresolved
normal retains the projectile without inventing a separation or damage event.
A newly approaching contact after separation can detonate when armed.

This is a bounded replacement policy. Exact Havok friction, angular motion,
material mixing and contact manifolds are outside its acceptance. The separate
authored bounce damping fields are retained in the validated catalog and are
not multiplied into motion without a proved consumer. Generic maximum age plus
native grace bounds projectiles that never reach an armed contact.

## Damage and lifecycle

One armed impact or age expiry produces one normal explosion. The selected
retail terminal path does not consume an extra direct-impact damage scalar;
the server does not add one. Character damage uses the existing qualified
Character callback and its own armor rules. Vehicle receivers preserve the
ammo-specific armor pierce and reduction as well as radial and material tests.
Logical objective damage is not inferred from an authored objective channel.

The launcher shares the established projectile slot, reliable queue, late-join
and cleanup machinery with rockets. Reservation/allocation and round identity
remain attached to the shot through the owner's death or respawn. Replaced
allocations cannot inherit it. Terminal publication and all prepared damage
record before installation; recorder refusal and queue pressure retain a
retryable flight without partially applying effects.

Synthetic and original-input checks establish the source behavior only. Native
rendering, packet arrival, ammunition HUD and live-match acceptance remain to be
reviewed separately. Releases remain manual after the maintainer's review.
