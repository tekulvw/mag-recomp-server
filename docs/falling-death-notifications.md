# Falling death notifications

An accepted retail on-foot self fall publishes the existing death notification
when it first takes a living human to zero HP. Source type Falling1 and signed
index -1 are distinct from ExternalKinetic damage kind5. The same admitted
character is both attacker and defender. Current human members of that actual
squad, including the falling player, each receive one notice.

The original retail death dispatcher sends before choosing incapacitation, so
the first collapse publishes whether the resulting life is incapacitated or
awaiting respawn. Finishing, bleed-out and repeated owner reports do not replay
that death notice. Existing falling amounts, modifiers, death/suicide counters,
XP and respawn handling remain owned by their existing consumers.

The fall planner admits only current living, on-foot self damage with the
qualified type, hit-location and source attribution. The pending hit retains
its existing reservation and generation checks. Notice planning, recipient
snapshot and semantic audit belong to the same owner-event transaction; the
application notice is queued after the recorded health/life commit succeeds.
Recorder failure preserves the character and queues and remains fail-stop.

Head flags use the optional configured retail combat body's location0 mapping.
The original mapping is non-head, and the ordinary on-foot path has no associated
vehicle. Synthetic tests remap the body to check the predicate independently.
With no optional combat body catalog, existing falling damage still works and
no head flag is invented; that configuration omits the notice.

Actual owner-packet regressions check squad recipients, initial incapacitation,
expiry, terminal/nonlethal falls, repeated reports, missing body metadata,
remapped head flags and failed recording. Original-file evidence qualifies the
retail metadata and downstream sender, not live packet delivery or presentation.
The displayed fall notice remains part of human live-match review. Releases
remain exclusively manual.
