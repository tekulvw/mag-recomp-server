# Manual live Ground vehicle repair

The authenticated retail Repair message now repairs ordinary source-owned
Acquisition APC/Escort and Domination APC/Motor Pool vehicles. It uses the current
source-bound equipped kit and effective RepairAmount modifier, current allocated
vehicle identity, actual owner team and live authoritative float HP. Acquisition APC repair also accepts existing schema-v1 source catalogs; it does
not require APC crew, driver, or turret bindings. No new
vehicle repair-time catalog is needed: the retail Ground virtual selects the
instant authored kit amount. Existing extraction supplies amount, cooldown and
item binding. Preserve the f32 fused amount calculation, percent conversion and
percent-to-HP round trip; do not substitute a direct integer-HP increment.

The supported server route requires client TCP/plugin1, a started open match,
a living ready on-foot actor, the current equipped kit and complete Ground
creation. The full family1 ID must resolve to a supported source allocation
(type3/9 Acquisition, type3/7 Domination), with a live operational non-hulk and
noncritical model on the actor's actual team. These restrictions describe the
replacement's supported ordinary authority. Native client target selection also
uses actual team and damaged HP. The native server target virtual itself is
constant true; no server-side center-distance, mount-point radius or replacement
raycast is invented. The client supplies its selected target.

One recorded transaction stages HP, effective leadership, the shared manual-tool
cooldown, optional repair work/stat35/XP receipt and field18 health publication.
The recorded candidate retains generation, source spawn/type, kit/modifier and
prior attacker provenance. Recorder or outbox failure leaves those states
unchanged. A positive float HP change can count even when its quarter-HP network
projection is unchanged. Credit uses the existing separate fractional work and
XP carries. A remembered attacker on the repairer's current team suppresses
credit while HP restoration still succeeds. Missing XP opt-in keeps gameplay
work/stat35 without emitting a new receipt kind. AutoRepair remains rewardless.

Repair preserves damage source, attacker, location/type and user data, pose,
body/collision, occupants, driver ownership, spawn and objective associations.
It queues only the ordinary field18 health delta. Native Heal dirties its model
for the following vehicle update; immediate queueing after this server's record
is replacement publication timing. The existing outer bridge drains and
resequences the reliable queue through its own delivery record. A later delivery
failure does not undo an already committed gameplay transaction.

Synthetic checks cover real Acquisition tool sourcing and outer bridge output,
Acquisition APC repair without crew bindings, its unsupported-source and authority
fences, modifier work and atomic retry, all three Domination APC faction bindings,
Motor Pool repair, current target-only
FRAGO, sender/plugin/route/state/tool guards, actual team independent of faction,
float/sub-quarter repair, effective modifier and work carry, full-outbox and
recorder rollback, cooldown and once-only retry. Exact retail evidence remains
private and reproducible; original files and populated catalogs stay external.

The native exceptional client 11-second and server post-stuck 12-second timestamp
paths are separate from ordinary tool cooldown and are outside this consumer.
The source does not invent an ordinary damage grace period or repair reset.
Neutral/special bodies, rebuilding, cross-family expansion and separate zero-XP
completion counters remain follow-through. Native transport/presentation and
live-match acceptance are pending. Source integration does not deploy the server
or cut a release; releases remain manual after human live review.
