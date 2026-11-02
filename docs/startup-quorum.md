# Ready-human startup quorum

Startup quorum is an operator policy for the configured match, not a claim about
native matchmaking rules. It applies before the shared world starts playing.

Horizon settings accept an optional `MagMinimumPlayersPerTeam` array in native
match-side order: team 0 (attacking), team 1 (defending). For example:

```json
{
  "MagMinimumPlayers": 2,
  "MagMinimumPlayersPerTeam": [1, 1],
  "MagMaximumPlayers": 8
}
```

This waits for at least one ready human on each side. `[2, 1]` requires at least
three humans in total. A larger `MagMinimumPlayers` remains an additional total
requirement. Omitted/null Horizon settings or `[0, 0]` preserve total-only startup,
including the existing minimum-one solo behavior. This change does not enable a
quorum in an existing deployment automatically.

Horizon advertises `max(MagMinimumPlayers, sum(MagMinimumPlayersPerTeam))` in the
existing status minimum-player fields and sends that total plus the same team
array in `configure_match.minimum_players_per_team`. The existing status wire
format has no per-side quorum fields; it continues to describe the minimum total,
not a promise that players on one side can start the match. Missing bridge arrays
default to `[0, 0]`. Arrays must contain exactly two nonnegative integers whose
sum fits maximum capacity. Gameplay additionally checks the loaded match's team
allocation, squad bindings and capacity, and refuses incompatible configurations
before Horizon opens queues. Nonzero team minima require automatic team assignment
and cannot be combined with the single-player development dummy.

Only current, claimed human reservations that have completed initial object
publication and report Ready count. The pending initial Ready is evaluated by the
same rule when choosing the initial GameState phase. Expired, backlogged, unclaimed
and unready reservations are excluded. Duplicate reservation account IDs or
supplied authenticated account names cannot inflate the counts. Bot registry
actors and development targets have no eligible human reservation and do not count.
A Ready withdrawal or departure before play keeps the match waiting.

Once playing, loss of startup quorum does not stop or restart the clock. Late
admissions join the running world. Releasing or expiring the last human resets
the world to waiting under its retained quorum policy for the next cohort.
`match_configured` and `match_started` receipts include per-team minima, and the
start receipt includes the actual per-team ready-human counts.

Synthetic shared-engine regressions cover same-side waiting, pending Ready,
readiness withdrawal, expiry, identity duplication, capacity rejection, legacy
solo startup, post-start quorum loss and the next cohort. Horizon's synthetic
check verifies matching advertised/bridge totals and validates settings. These
checks do not establish native menu presentation or multi-computer acceptance.
