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
other context can still satisfy the source OR predicate. Family 33, repair/objective FRAGO families and leader tagged-completion stat 41
remain separate work. Teammate healing and revival use the support path below.

Synthetic checks cover calculation, geometry, source binding, kill capture,
rollback, immutable replay, zero and current-round publication. Private static
checks bind the exact retail executable, archive and source map graphs. Native
client presentation and live-match acceptance remain pending. Source integration
does not authorize deployment or a release; the maintainer releases after review.

## Teammate healing and resuscitation

Support bonuses reuse the same geometry and atomic receipt/ACK path. At a
credited application, either the healer's current position or the resolved
patient's current position can qualify against the healer's current squad tag
or primary objective. Healing uses positive whole HP restored after the actual
quarter-HP application, independently truncated per tick. Revival uses one unit.
Shared fractional XP is calculated before applying the authored bonus, so an
in-range application with zero whole base XP returns an immutable zero bonus.

Both support actions suppress earned credit when the patient's remembered last
attacker resolves to the healer's actual match team. Revival still restores HP,
clears incapacitation, preserves damage attribution and sends the notification;
only its stat34 and XP receipt are suppressed. Source object identity does not
substitute for the remembered attacker. Self-heal and leadership AutoHeal do not
produce support XP or FRAGO receipts.

The separate `frago_action_bonus_v1` Hello capability negotiates optional
`frago_action_bonus_enabled`. Horizon emits it only to a supporting peer and
retains the enabled value with the reservation. It requires the authored bonus
coefficient and resuscitation rule; a zero authored rule is valid. Contextual
support from a reservation that was not enabled is rejected before settlement.
Kill negotiation remains independent. New gameplay with old Horizon receives no
support opt-in and sends ordinary support receipts; new Horizon omits the new
field for old gameplay. A fresh contextual revival fails if its authored rule
is subsequently absent, but exact historical replay returns the saved response.

Deferred healing resolves the saved healer against the full current reservation,
world, player, account, character and round before attaching a FRAGO context.
Disconnected or replaced identities retain the existing base-credit policy and
receive no newly sampled bonus. Accepted owner-update patient poses precede
queued healing in the same transaction. The existing bounded catch-up scheduler
can settle several ticks with that update's current context; it does not invent
historical range samples or claim native timer parity. The native effect saves
only a packed source-object ID; its allocator reuse behavior remains unqualified.

The schema3 saved receipt representation is extended to accept contextual heal
and revive kinds. **A pre-support Horizon binary cannot read a store containing
these new contextual receipts on downgrade.** Back up and migrate such stores
before a rollback; do not rewrite history to remove immutable receipts. Existing
stores and ordinary support receipts remain readable by the updated server.

Connected synthetic tests cover patient-only and healer-only range, accepted
current movement, capability omission/false, exact identity replacement,
record failure, repeat events, immutable ACK/stat3 publication and friendly-damage
revival. Live in-match presentation and acceptance remain pending. Repair's
separate fractional work counter and objective-action contexts are next gameplay
work; restored HP must not be substituted for repair units.
