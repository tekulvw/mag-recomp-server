# Acquisition world queries and lifecycle consumers

Build the collision and lifecycle behavior needed for a playable Acquisition
loop. Exact native solver bits and broadphase order are fidelity limits, not
additional release gates. Keep original evidence/resources private, existing
services and game sessions untouched, and releases manual.

Extend the existing convex support solver to return bounded closest distance and
normal, then conservatively advance the actual character capsule against static
triangles and current dynamic roots. The concrete provider implements the
existing spawn-blocker contact/cast/move interface on an isolated owned position.
Require current full collision filters, explicit static filter/radius metadata,
complete world coverage and bounded work. Missing data must error. Starting
contacts may escape when motion separates; do not treat every overlap as an
unconditional fraction-zero hit. Exclude the exact spawning root and self.

Keep immutable ground resources separate from mutable current pose/health/state
authority. Add qualified existing-Escort respawn/death/delivery consumers and
ordinary immediate character damage consumers using current death, reward and
notification logic. Never substitute an owner-update queue, unconditional kill,
new APC allocation or an invented timer. Where a command needs unavailable
source parameters, expose that requirement explicitly.

Prefer this concrete provider over the existing AABB sweep: the AABB has a
different shape and cannot resolve the actual capsule/vehicle contacts. Avoid
porting an opaque native physics engine; the existing bounded convex solver and
neutral geometry are maintained replacement source. Mines remain unavailable in
the Engine until owned admission/explosion effects are implemented; supporting
their template alone does not enable them.
