# Contextual FRAGO kill bonuses

Source-owned Acquisition and Domination can now add the authored FRAGO bonus to
an existing, authenticated enemy-kill reward. The server reads the credited
killer's squad at the event, checks its effective full tagged ID and distinct
primary objective, and qualifies when either the killer or victim context is
within the target's radius. Distance uses f32 XYZ and an inclusive boundary.
Pooled identity/existence matters; objective activity, target allegiance and
vehicle HP do not introduce additional range gates.

The eligibility fact is frozen as optional private `frago: true` on the existing
one-unit `enemy_killed` receipt. Retagging, movement or retry cannot recompute it.
The gameplay Hello advertises `frago_kill_bonus_v1`. Horizon includes
`frago_bonus_enabled` only for a peer with that capability, and enables it only
when its selected retail progression catalog supplies the authored coefficient. Missing capability or geometry retains
ordinary XP behavior. A present contextual receipt requires qualified settlement
rules and an immutable `match_frago_points` acknowledgement, including zero.

Horizon first computes ordinary whole XP and its shared fractional remainder.
It truncates the f32 product of positive whole XP and the authored bonus
coefficient, adds only a positive bonus, and records cumulative match stat 3
alongside Battle XP. Points, carry, account progression, finalization and the
immutable receipt remain in the existing atomic store transaction. Ordinary
level and saved leadership eligibility use the resulting account XP. Rust records
the acknowledgement before publishing stat 3, ignores older cumulative values,
and keeps old-round receipts from altering new-round totals or another character.

## Source data preparation

Use `tools/extraction/project-retail-frago-rules.py` with an existing source-gameplay
objective catalog, its exact authored map catalog, the selected extracted
progression catalog and the owned original retail ELF. Supply `--objective-catalog`,
`--authored-catalog`, `--progression-catalog`, `--retail-elf` and a private `--output`;
`--check` reproduces exact output bytes. Original inputs and populated outputs
remain external to the public source tree.

The projection preserves the catalog and adds complete geometry bound to the
selected map, mode, source image, authored catalog and progression input. Explicit
zero radii stay zero. Missing definition values use the verified constructor
default read from the exact ELF. Vehicle range comes from the authored XP table.
Existing interaction/demolition positions take precedence over source instance
translation. Current Ground, Helo and Bunker target poses come from their owned
runtimes. Update normal objective-file hashes in dependent ability map bindings
when preparing a new profile; do not silently accept a mismatched pin.

These anchors are the replacement server's current source-derived positions.
Native terrain snapping and placement parity remain separate fidelity work.
A missing current pose or unsupported family cannot qualify by guesswork.
Ground-mounted character contexts resolve their current vehicle root; other
unqualified mounted character roots are omitted from that position test. A known
other context can still satisfy the source OR predicate. Family 33, non-kill
FRAGO families and leader tagged-completion stat 41 are separate work.

Synthetic checks cover calculation, geometry, source binding, kill capture,
rollback, immutable replay, zero and current-round publication. Private static
checks bind the exact retail executable, archive and source map graphs. Native
client presentation and live-match acceptance remain pending. Source integration
does not authorize deployment or a release; the maintainer releases after review.
