# Hospital rocket gameplay implementation

**Goal:** Enable the three source-bound hospital secondaries through equipped
launch, current represented-world sphere contact, ordinary on-foot Character
damage, terminal publication and reliable cleanup.

**Baseline:** Fresh public main `3e100a4875b1c9fb444ebd113a185b41fd78c665`.
Both SDK mains were fetched; the clean public SDK was fast-forwarded and unrelated
private/native state was preserved. The human authorized autonomous parallel Sol
implementation and integration after focused checks. Releases remain manual.

## Design

Optional `core.hospital_rockets` requires the existing hospital turret role input
and explicit `core.projectile_contacts`. Load and validate both direct and registry
paths before a match can use them. Contact rules bind the static-world bytes,
dynamic catalog, sequence pack/neutral/part identities and operational
hospital/Helo source assets. Current overlays compete with static candidates.
The declared represented reference pose preserves provisional static metadata;
it is a replacement policy where compiled parent transforms are unavailable.

An actor-owned four-slot rocket book shares native wire identities with frag
grenades. A flight pins reservation, world, round, match, Character, team, source
weapon and projectile ID. It continues across dismount/death/respawn within that
reservation, and release/idle retires it before Character removal. Exact identity
guards permit existing current-reservation consequences without transferring
credit to a reused slot. No cross-disconnect projectile continuation is claimed.

Launch requires authenticated route/shooter, current exclusive hospital grant,
versioned secondary readiness and primary fence. Server-owned cadence and
magazine state survive remount. The source muzzle chain supplies the current
origin; finite normalized request direction, full power and no inherited motion
are explicit bounded replacement choices. All authored timings and amounts come
from the external rules. The ordinary grenade path remains separate.

Motion, terminal, on-foot damage and publication form detached recorded candidates.
Ignore owner rocket position/terminal claims. At contact, use current body/stance,
one layer-24 filtered material ray and the typed Character dose through existing
armor, incapacity, statistics and reward machinery. Do not route vehicle dose to
infantry or add an unproved direct-impact callback. Rocket kill-feed weapon indices
remain deferred. Publish state2 before inactive removal and retain the shared slot
until removal is queued reliably. Capacity errors defer candidates while draining
the shared FIFO; unavailable geometry never becomes a clear segment or stalls the
match indefinitely.

## Ownership and checks

- Launch owner: actor book, grant/cadence, event planning, motion/tick lifecycle,
  shared frag conflicts, late join and release cleanup.
- Damage owner: shared native radial armor/falloff helper, typed Character request,
  current on-foot overlap/stance/filtered material collector and source guard.
- Scene owner: source-bound contacts, current overlay enumeration and one-budget
  combined query, exact Character exclusion and qualified phantom handling.
- Root: central fields/defaults, configuration/profile binding, source staging,
  connected normal gameplay verification and exact checked source integration.

Observe focused behavioral RED/GREEN. Exercise slot3 creation, forged owner reports,
actual scheduled contact, one damage/credit/terminal, cleanup and reuse; both frag
conflict directions, stale grant/source/victim, post-death continuation, release,
queue pressure and recorder rollback. Verify current transforms and explicit
coverage/work failures. Run the affected full suites, normal CI and original six
profiles; distinguish source replay from native client/live-match acceptance.

## Verified source checkpoint

- [x] Equipped launch, separate four-slot flight state, shared frag conflict guards,
  current Character association, record-first motion/damage and reliable cleanup.
- [x] Source-bound static/current scene, exact-part bindings, explicit reference
  scope, current on-foot typed damage and frozen identity checks.
- [x] 1,041 Rust tests in36 suites,294 Python tests,fmt,strictClippy,builds and
 1,152-file public boundary.
- [x] Final six-profile original-data replay matched252 product source files and
 260 inputs, with lethal damage, kill credit, terminal/inactive delivery and reuse.
- [x] Both six-profile startup scopes and malformed rocket source rejection.
- [ ] Normal source CI, exact checked-tree integration and private checkpoint.

The replay uses scripted positions and leaves the separate optional
incapacitation catalog disabled. Native input/live-match acceptance is pending.
Ground vehicle rocket damage and ordinary fourth-slot frags follow as core work;
kill-feed indices and remaining native fidelity remain separate.
