# Acquisition controller planning

Build a typed, external four-chain controller and a pure command planner. This
is the next increment after APC insertion, authorized by the user's go-ahead.
The existing Rust server remains the implementation language; original map and
script data stays private. Use a generic bounded condition/command schema rather
than compiling an original script or map-specific branches into Rust.

The projector parses a restricted, nonexecuting script grammar from explicitly
supplied original resources, joins its parameters to qualified map evidence and
the selected spawn catalog, and emits external ordered conditional groups. Keep
raw damage-volume arguments/symbols and unbound trailing calls explicit. Do not
correct source anomalies or infer an objective-state callback schedule.

The validated Rust catalog is bound to the exact spawn bytes and map. Four chains
must cover two platoons/two stages with distinct known objective, vehicle-spawn
and volume tokens. Commands may address only the declared target family.
Planning requires a complete explicit current-state snapshot; unknown states
remain Other, while missing observations fail. Each group's first matching
branch wins; all groups run in input order. No state, account or wire mutation
occurs. Native callback timing, objective pool IDs, command application, arrival,
destruction and scoring remain separate bindings before gameplay integration.

Tests use invented data. Private verification reproduces all three original map
graphs for both opposing attackers and exercises them through the actual Rust
loader/planner. Source integration uses a focused checked public PR and private
main checkpoint; releases and released pins remain manual after live review.
