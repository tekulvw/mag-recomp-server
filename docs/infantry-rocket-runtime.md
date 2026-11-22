# Stock infantry launchers

The optional retail launcher path supports the three faction Heavy-kit weapons
projected by `tools/extraction/project-retail-infantry-rockets.py`. Keep the
generated catalog with the owner's private runtime resources. Set
`infantry_rockets` to its path in Acquisition gameplay configuration, or
`core.infantry_rockets` in Domination configuration. Relative paths resolve
against that configuration file. Omission keeps the path disabled.

Startup binds the catalog to the loaded retail item/combat provenance, actual
weapon and ammunition definitions, faction and any installed starter kits.
Acquisition requires its explicit projectile-contact binding, Structural scene,
dynamic bodies and classified Ground areas including Escorts. Domination's
first supported configuration uses the existing combined Hospital-rocket
receiver profile, including its Ground and Hospital resources and Helo
receivers when that domain exists. Infantry-only Domination configuration
without that combined receiver setup is outside this increment.

Use `--validate-catalogs` to check the resources without opening a listener.
Readiness reports `infantry_rockets_validated`; readiness does not establish
native client presentation or a completed live-match test.

## Launch and ammunition

Authenticated retail `0210` requests select the current equipped gear-3 item.
The owner must be alive, entered, initialized, on foot and in an open round.
The shared projectile slot must be free of active frags, rockets and pending
cleanup. An exact active-flight retry does not spend another round.

The finite allowance comes from the weapon's configured magazine capacity and
initial magazine count. Refire uses the authored interval. Native reload
presentation and timing remain client-owned. An authorized static supply pulse
restores spent launcher allowance in the same recorded candidate as its timer,
frag refill and health effect; it preserves active flights and refire timing.

The traced retail creation path consumes launch position, quaternion and
additional velocity from the authenticated client's request. This
implementation treats those spatial inputs as client-trusted while validating
finite rigid/wire values.
It does not independently bind the reported muzzle or inherited velocity to the
actor's physical pose. The server computes launch power with the effective
`kCharModGrenadeThrowPower`, adds the reported velocity and owns subsequent
flight queries, terminals and admitted damage.

## Flight, effects and lifecycle

Each flight uses the authored capsule with its fixed launch orientation.
Queries sweep translation against the selected represented contact scene and
current admitted dynamic bodies. The terminal blast uses the projectile's
reference/root position. An earlier armed contact takes priority over the
strict sampled displacement/range check; the configured lifetime and native
age grace provide the remaining bound.

Retail pre-arm contact bypasses impact detonation, but its physical continuation
is not reconstructed here. A blocked pre-arm flight is retired without blast
as an explicit unsupported-response limit. It does not pass through the
obstacle or apply a fabricated bounce. Fixed orientation and scheduler sampling
are the current replacement scope, not a claim of full native rigid-body parity.

The separate Character and vehicle damage channels use the existing classified
receivers and material tests. Acquisition includes operational roles 3, 7 and 9;
Domination uses its Ground, Hospital and optional Helo receivers. Logical
objective damage is not inferred from a vehicle or Character channel.

Creation, ammunition and terminal effects record before installation. The
source retains its original reservation/allocation and round; death or respawn
within that allocation does not transfer credit. Released or replaced sources
cannot inherit a flight. Late joins receive active creations, reliable queues
defer terminals when full, and cleanup waits behind undelivered terminal state.

Source and offline checks remain separate from native packet-arrival, visual,
HUD and live-match review. Releases remain the maintainer's manual action.

With the optional retail firearm-assist rules enabled, positive effective
Character damage from infantry rockets and hospital secondary rockets shares
the firearm and ordinary-frag stat-11 ledger. The projectile's launching
allocation retains attribution through its owner's death or respawn. Each
victim's first collapse settles prior eligible contributors once. A blast's
Character HP, assist ledger, statistic publication and any negotiated human
assist XP receipts are installed together after the terminal record.
See [assist credit and experience](firearm-assist-credit.md).
