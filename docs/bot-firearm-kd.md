# Bot firearm kill and death accounting

The optional world-owned bot population keeps absolute enemy kills (stat10)
and deaths (stat12) per allocation. Its ordinary admitted firearm paths use
current actor/team and life identity. Account XP remains confined to existing
legitimate human reward recipients.

Human-to-bot hits count the registered bot's first lethal transition. Bot
scheduler hits count an opposing shooter's kill and the victim's death in the
same recorded shot. A human's first incapacitating collapse uses the existing
human death statistic; finishing hits and expiry cannot add a second bot kill.
Friendly bot victim deaths still count, without an enemy-kill increment.
Generic damage helpers, environmental damage and other weapon classes do not
silently gain firearm statistics.

A bot's immutable allocation generation is separate from its current life.
Respawn preserves totals, while retirement, replacement and round reset discard
them. The accounting plan revalidates the world, round, allocation, current
life and previous totals. Damage/owner/stat packets are encoded before the
owning event is recorded; installation follows admitted damage.

## Live publication

An enqueued Character creation is not yet a published Character. Each recipient
therefore retains a fence at the queued allocation snapshot's FIFO position.
Only successfully recorded and consumed shared replication advances the fence.
The shared drain can send the current absolute bot totals after that creation,
even while unrelated later frames remain queued. A later join gets its own
creation fence and current totals. Retirement discards old allocation tickets.

Statistics are application0204 packets, not ObjectUpdates. They consume the
existing reply message/byte budgets but neither replication sequence numbers
nor NAK history. The drain alternates eligible statistic rows and replication
frames, rotates recipients and bot slots, and records the complete publication
before moving queues or sent baselines. A full reliable recipient queue retains
the existing recorded retirement policy; healthy recipients continue.

## Verification scope

Synthetic regressions exercise authoritative human Hit and bot scheduler routes,
nonlethal/stale inputs, incapacitation, record refusal, respawn and allocation
reset, plus partial creation drains, latest-value publication and late join.
External source-data replays are retained privately. None of these checks claim
native scoreboard presentation, client application acknowledgement, assist
attribution, final-results bot rows, resupply or mode-objective bot AI.
Releases require the maintainer's live-match review and manual action.
