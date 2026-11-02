# Atomic Acquisition callbacks

Implement the callback adapter as an ordered isolated transaction over a required
effect backend. The backend stages authoritative entry/FSP/vehicle effects,
supplies a complete current-state snapshot, applies every ordered script command
and trailing call, then recomputes assignments and validates publication. Missing
or incomplete effects fail the preparation; no partial live-state application.

The adapter records the exact staged audit and output before infallibly installing
the candidate. World incarnation, revision and monotonic callback IDs reject
stale/mismatched requests; bounded cached replay never repeats effects. A backend
stage must own its mutable candidate and perform no external writes/publication.
These semantic requirements are explicit; cloning shared mutable authority is
not a valid isolated stage. No backend is installed implicitly.

Keep current-state predicates separate from target requests. Preserve pre-script
entry effects and post-script assignment refresh. Native DamageVolume requires
complete ordered collision-pair processing, not a guessed/deduplicated victim
list. Unsupported physics/actor/vehicle/mine effects remain errors. Existing
pre-MissionStart catalog qualification cannot enable an interactive backend.

Bind the trailing score-rate setter as its own primitive: explicit admitted input
domain, native unsigned deadlines, preserve earlier deadlines on repeated calls,
zero disables, strict due comparison and one deduction per Update. This does not
implement Acquisition winner/end rules or reuse bulk elapsed-time catchup.

Public tests use invented effect worlds. Original graphs are checked privately
against the adapter, with unsupported effects demonstrably leaving state intact.
Integrate source through checked PRs; preserve services, released pin and manual
releases. No live gameplay/physics compatibility follows from these unit checks.
