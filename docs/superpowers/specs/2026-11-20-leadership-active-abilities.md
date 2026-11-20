# First playable retail leadership active abilities

The approved milestone is a mode-enabled squad strike and sensor support through
the normal authoritative server path. Implement ClusterBomb and Sensor first;
retain explicit unsupported results for other command and strategic effects.
The existing replacement server, source-qualified mode profiles and retained
leadership codecs are the baseline. Integration is opt-in. Releases remain
manual after the maintainer's live-match review.

## Approach and scope

Extend the current Rust Engine and reuse its character/vehicle damage,
collision, replication, roster and durable transaction boundaries. A separate
ability service or a diagnostic-only command bypass would duplicate authority
and would not satisfy the normal in-game request path. Extract configuration
from original retail inputs and qualify native interpretation independently;
development symbols may guide research but do not establish retail decisions.

The initial roles are the already implemented native squad leaders. Permit that
policy in profile-backed matches, without granting account roles or treating an
arbitrary first member as an authorized active-ability user. Platoon/OIC election
and strategic execution remain separate work. Retain existing passive behavior.

## Required behavior

- Bind requests to the authenticated actor, held role, actual squad/platoon,
  selected formation, game mode, side and authoritative server clock.
- Select authored availability through the qualified native mode initializer;
  never substitute the configured human limit for the retail match format.
- Apply native target, tactical-asset, enabled-state and anti-air predicates.
  Distinguish an invalid request from a native accepted activation whose effect
  fails; do not guess which path consumes timers or publishes failure state.
- Keep shared platoon and individual ability timers in round state, independent
  of reservation, death, leader replacement and reconnect. Reset at actual round
  retirement/fresh selection, not an idempotent selection retry.
- Publish qualified native ability state and late-join snapshots. Do not invent
  a success packet when native behavior uses replicated state instead.
- Schedule original strike work and sensor scans with their native per-update
  limits, randomness, target rules and cleanup. A uniform damage radius or an
  omniscient radar flag is not a substitute for these mechanics.
- Resolve damage through current collision/material and armor/death authority,
  including supported bots and vehicles. Apply source team rules consistently.
- Prepare all fallible work before recording and installing state. A failed
  record must not consume a cooldown, publish activation, damage an actor or
  leave a partly scheduled effect.

## Data and acceptance

Public changes contain replacement source, neutral extraction tools and invented
fixtures only. Original game resources, populated catalogs, disassembly and
private comparison receipts remain external. Missing source-bound prerequisites
fail clearly. Existing no-ability configurations and unrelated modes keep their
current behavior.

Meaningful synthetic tests cover authorization, per-unit cooldowns, duplicate
transport handling, replacement/requeue, failed recording, bounded scheduling,
effect resolution and late join. Private offline consumers replay qualified
retail configuration through the normal Engine path. Source checks do not prove
native client presentation, live compatibility or performance. No game launch,
service change, deployment or release is part of this source increment.
