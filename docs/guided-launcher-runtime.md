# Optional human guided launchers

The source-bound `mag_retail_guided_launchers_v1` catalog enables Skill 25's three
faction launchers in Acquisition and Domination. Configure its external path as
`guided_launchers` or `core.guided_launchers`, respectively. The existing stock
`infantry_rockets` catalog is independent and remains byte compatible. Neither
catalog is implicitly enabled. The guided catalog validates original equipment,
required sights, ammunition, finite counts, motion values and source digests;
its native proof pin binds motion, ordered-update and network qualification.
The companion projector documentation describes the external inputs.

The launch path reuses authenticated current living on-foot ownership, gear 3,
required sight slot 21, owned Skill 25, native0x0210 bytes, four common projectile slots,
refire fencing, duplicate detection, source collision and existing typed blast
receivers. Three guided rounds have their own current-life budget, independent
of stock and underbarrel reserves. Static resupply refills that same budget.
There is no additional reload deadline beyond the existing authored refire
policy. Failed recording or shared-publication capacity leaves admission,
ammunition and warnings uncommitted.

The shot captures its own0x0210 target. A prior0x0229 state 2 warning is unnecessary;
a later aim report cannot retarget a flight. MAX selects the legitimate unlocked
RocketDumb branch. Other unsupported targets, including family 33, are refused
rather than converted into an invented no-lock shot. Represented handles admit
the low 16-bit index and known active bit; other middle-bit aliases are outside this
replacement's scope. Target identity binds the current actual match world,
round/incarnation, authored source scope and entity allocation. Ground uses its
existing entity generation, which ordinary respawn does not increment. Helo and
Hospital retain their authored allocation across flight cycles and rebuilds.

With `anchor_policy=retail_constructor_seed`, the current vehicle root receives
world-Y offsets Ground +0.8 m, Helo −2 m and Hospital +1 m, before the separate guidance
bias. The reflected placement offset is not the guidance member. Later native
writes to these constructor seeds remain a fidelity boundary. Missing current
allocation/pose or a strictly rearward target permanently latches steering loss.
Loss keeps heading and does not switch propulsion, reacquire, or release the
flight's warning. A degenerate biased direction explicitly holds heading and
bias as a bounded replacement choice; launch separation zero is refused.

The motion module implements the verified ordered algebra: initialization draws
an interval and records unbiased initial target distance; decisions use the old
bias, replace the next bias with `max(.5,.1*distance/initial_distance)`, and apply
the perpendicular .07 cap using the normalized .93/.07 blend. Cadence is sampled
from .01–.10s. An already negative clock is skipped; a due clock discards
overshoot and installs the new interval before its one direction decision.
Unlocked Dumb preserves its initial heading through the strict 2 m cruise boundary,
then samples two signed .005 transverse perturbations in the verified order.
Even fixed intervals and magnitude bounds consume their random draws.

Replacement stepping uses at most 1,201 substeps of 10 ms. Each original-style
substep has at most one decision; several substeps can decide within an outer
world tick. Current-root strict age/range checks precede steering and sweeping.
The active guided/unlocked age limit is 10 s; the separate two-second native age
validator allowance does not extend this active limit. Stock flight timing is
unchanged. Guided reads the source global 150 m range; Dumb reads its ammo range.
The latest committed target pose is sampled for the outer tick. Initial velocity
retains launch power's f32 FMA modifier and extra velocity; a due decision resets
velocity to heading×60 m/s. Capsules and peer snapshots follow current heading.

A deterministic per-flight xorshift32 stream is seeded from recorded owner
allocation, round, player, slot and launch time. Samples use low 30-bit bits converted
to f32. This preserves owned state and draw order, not the original per-thread
PRNG sequence or bit-identical SIMD/Havok scheduling. The existing deterministic
collision provider chooses earliest represented contacts. Current-root age/range
checks win before that substep's sweep. Native initial contact timers are .1s
and zero. Pre-arm contact conservatively retires without damage, bounce or
unproved Dampen behavior. Existing Character/Ground/Helo/Hospital damage
qualifiers and record-first terminal cleanup remain authoritative; authored
objective damage alone does not enable an unqualified logical objective route.

Native0x0229 is a separate seven-byte authenticated aim warning: opcode, big-endian
target and full state byte 0–2. State 0 must name a real represented vehicle. The
payload supplies no shooter. The server adds no lock-time, team, LOS or range
requirement and never uses the warning state as a launch gate. This bounded
owner route requires current living on-foot readiness and an existing Character.
Aim contributions and live guided-flight state 2 contributions remain separate;
aggregate priority is2, then1, then0. Repeated reports do not duplicate counts.

Warning candidates combine changed fields into one ordinary native ObjectUpdate
with canonical family 1/2/13 lists and shared Vehicle field 24 u8. Ground/Helo/
Hospital lists retain 30/27/25 fields. An observer receives only families whose
base creation is known. Per-observer allocation state tracks the last queued
warning, allowing a later observer to receive the current nonzero state without
replaying aim contributions. It is not a confirmed-network-delivery receipt.
Owner events stage combined warning changes before their record. World ticks
batch changes after all projectile terminals, reserving a future warning frame
in terminal publication checks. Ordinary immutable shared FIFO delivery can
span multiple bounded drains; warnings are not a direct fanout batch.

Death/downing, mounting, new spawn, owner removal, round changes and stale target
allocations prune aim ownership. Flight contributions derive from the existing
recorded shared-slot state and survive steering loss until terminal/retirement.
Speculative damage callbacks do not mutate a second warning counter. Refused
records retain the original slot and contribution. No warning grants damage,
health, XP or equipment authority. Native presentation, warning audio/rumble,
exact dirty-field fanout and timing still require human live review. Releases
remain manual.
