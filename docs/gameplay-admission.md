# Gameplay admission over the bridge

A join reserves its seat on the gameplay worker with one `reserve` request over the
JSONL bridge. Every bridge request opens its own TCP connection to the worker's
address. No bridge connection stays open between requests, so none can go stale
while the server is idle. The world stream is the one long-lived connection, and
its heartbeats keep it in use.

## Idle bridge

Horizon caches the worker's resolved address. An answer older than 30 seconds is
still used. Horizon resolves the name again in the background, and the next
answer replaces it. Before, the first request after an idle spell resolved the
name again inside its two-second deadline. One lost lookup took the resolver's
five-second retry, and that join failed. If a connection to an expired answer
fails, Horizon resolves the name again and reconnects within the same deadline.
Nothing has been sent at that point. If a connection to any answer fails, Horizon
forgets that answer, so a moved worker is found again. A lookup that a request
stopped waiting for still runs to completion, and its answer still fills the cache.

## Retrying an unanswered reserve

A reserve that the worker did not answer, because of a timeout or an unavailable
connection, is retried. A definite refusal is never retried.

- If the reserve never left Horizon, it is sent again at once. The name did not
  resolve, or no connection opened. The worker cannot hold it, so nothing is
  released.
- If the reserve was sent, Horizon releases that reservation first, and the worker
  acknowledges the release. Then Horizon reserves again under a new reservation.
  If the release is not acknowledged, the join stops retrying.

When every attempt goes unanswered, the reply depends on how the player joined:

| Join | Reply | Effect |
| --- | --- | --- |
| Queued: faction matchmaking or the [configured registry](configured-world-registry.md) | `6` | The ticket returns to the queue. The player keeps receiving waiting updates and can cancel. The queue offers the ticket to a world again. |
| Direct legacy join | `2` | The ordinary failure. |

A queued party follows the same rules with `reserve_party`. Each party gets its
own attempt limit, and each exchange gets the same deadline. A sent but unanswered
cohort is released by `release_party`. That release also fences a cohort the
worker never recorded. The party is then reserved again under a new cohort and new
reservation IDs. If every attempt fails, the party's members get no reply. The
party returns to the queue, and its formation partners return to the queue too.

No reply has been recovered that moves the retail client from its queue screen
back to the lobby. A direct join that gets `2` therefore still leaves the client
in its queue. The retries make that outcome rare, but they cannot remove it.

## Settings

| Variable | Default | Effect |
| --- | --- | --- |
| `MAG_GAMEPLAY_ADMISSION_ATTEMPTS` | `3` | Reserve attempts per join, 1 to 5. Attempts that were sent and attempts that were not share this limit. |
| `MAG_GAMEPLAY_ADMISSION_DEADLINE_MS` | `2000` | Reply deadline for each reserve, 500 to 10000 ms. |

An invalid value stops Horizon at startup. The deadline bounds a single exchange.
A direct join waits for its reply inline, so each extra attempt delays that reply
by up to one deadline. Raising the deadline does not replace the retries.

## Logs

- `Gameplay admission not delivered: ... sending it again over a new connection`:
  a reserve that never left Horizon was sent again.
- `Gameplay admission refused: ... attempt=N/M`, then `Gameplay admission
  retrying`: a sent reserve went unanswered. Horizon released it and is trying again.
- `Gameplay admission unanswered after N attempt(s): ... result=6|2`: every attempt
  failed, and the join got the reply described above.
- `Retail party admission not delivered`, `Retail party admission retrying` and
  `Retail party admission unanswered after N attempt(s)`: the same three cases for
  a party.
