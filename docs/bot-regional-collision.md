# Regional bot collision guard

An optional `regional_collision` object in a `mag_bot_population_v1` input
restricts Acquisition bot physics to one source-bound region. The existing
behavior is retained when the object is absent.

The object contains five lowercase SHA-256 strings: `world_sha256`,
`spawn_sha256`, `ground_sha256`, `dynamic_catalog_sha256`, and
`structural_clv_sha256`. Its `min` and `max` are finite world-space XYZ
corners, with a positive span of at most 128 m on each axis. It is accepted
only for application 20231 and Acquisition. Every input must match the loaded
resource, and a pre-existing verified static physics region (from the world or
its [coverage sidecar](collision-coverage-sidecar.md)) must enclose the whole
requested region. The guard does not grant static coverage.

A loaded runtime navmesh must bind to that collision world, fit inside the
region, and contain no drop links. An offline reference mesh remains an
offline reference mesh. Original Alaska data still needs separate static
movement qualification and a bounded runtime bake before this option can be
enabled there.

Before each bot candidate, the scheduler captures current unfiltered Ground
and hulk roots plus installed Structural collision bounds. Known standing
Characters use the existing actor grid; other represented bodies require
qualified state and bounds. Missing resources, stale snapshots, unknown
stances, or an unqualified carrier hold movement. Geometry is shared through
an immutable scoped view; the loaded world's metadata and other collision
consumers are unchanged.

Every actual body sweep and ground-support ray checks the region and current
obstacles. This includes intermediate step-up, slide, and fall queries rather
than only the observed start/end poses. Allocation, respawn, idle physics,
and ordinary movement use the same guard. A rejected candidate does not
install its pose, cursor, health changes, reservations, or movement packet.
The scheduler consumes the rejected interval so a later clear tick cannot
produce a catch-up jump. Existing route expiry still applies.

Acquisition spawn-blocker callbacks remain authoritative. Admission requires
every source spawn chassis to be disjoint from the region expanded by the
complete bound Character-root reach. Stored FSP and direct respawn poses must
match source creation poses. These checks preclude a protected bot from being
selected by that displacement path without suppressing a mandatory callback.

This is a conservative movement guard, not a mutual vehicle collision solver.
A vehicle may enter a bot's existing pose; the next candidate is held rather
than inventing a contact response. Native parity, production data admission,
and live-match behavior require their own validation.

Synthetic checks cover current-vehicle hold/resume without catch-up, exact
candidate rollback, stale and missing providers, Character stance denial,
operational-to-hulk replacement, and spawn-clearance rejection. The hulk test
uses the normal destruction lifecycle and a different offset hull.
