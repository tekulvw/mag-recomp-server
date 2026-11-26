# Bridge reply budget

Every reply from the gameplay worker to Horizon carries queued shared frames
for any ready recipient. That covers each world frame (20 per second) and each
event reply. Two budgets exist.

## Legacy shared budget (default)

A reply carries at most 128 messages (32 in a world of at most 8 players) and
24,000 payload bytes, and every recipient shares them. The drain gives each
recipient one frame per pass, so with 32 recipients each one receives at most
four frames per world frame, which is 80 per second when clients send no
events. Horizon refuses a reply above 128 messages, 24,000 payload bytes or
131,072 response bytes.

## Per-recipient budget (`--per-recipient-reply-budget`)

The worker flag gives each recipient its own allowance in every reply: up to
16 shared messages and 6,144 payload bytes. The reply as a whole carries up
to 1,024 messages and 262,144 payload bytes. Direct replies to an event keep
the legacy bound, and the round-robin order between recipients is unchanged.

With the flag, the worker advertises `reply_budget_per_recipient_v1` in its
`hello`. A Horizon that has seen that capability in the worker's latest
successful `hello` accepts replies and world frames up to 1,024 messages,
262,144 payload bytes and 1,048,576 response bytes. Without it, Horizon keeps
the legacy bounds. A Horizon that predates the capability refuses the larger
replies, so deploy Horizon before turning the flag on. A restarted worker
answers a new `hello`, and its capabilities set the bounds from then on. A
world subscription keeps the bounds that were in force when it opened.

## Measured effect

Headless sims on private inputs at authored capacity, with 32 observers and
no client events, so world frames alone drain the queues. Other settings:
900 simulated seconds, Apple M4 Pro, `nice -n 15`, one sim at a time.

| World | Flags | Evicted | Deepest queue (frames) | Per-recipient messages/s | Per-recipient KB/s | Drain mean (ms) | Tick mean / p99 (ms) |
|---|---|---|---|---|---|---|---|
| Acquisition 128 | neither | all 32 at 2.1 s | 512 (bound) | - | - | - | - |
| Acquisition 128 | coalescing only | all 32 at 2.0 s | 512 (bound) | - | - | - | - |
| Acquisition 128 | budget only | all 32 at 96.9 s | 512 (bound) | - | - | - | - |
| Acquisition 128 | both | none | 18 | 24 | 3.3 | 0.25 | 2.04 / 6.21 |
| Domination 256 | neither | all 32 at 1.9 s | 768 (bound) | - | - | - | - |
| Domination 256 | coalescing only | all 32 at 1.8 s | 768 (bound) | - | - | - | - |
| Domination 256 | budget only | all 32 at 2.3 s | 768 (bound) | - | - | - | - |
| Domination 256 | both | none | 53 | 31 | 6.5 | 0.52 | 6.96 / 12.06 |

The two flags are needed together. With the budget alone, a recipient
drains 16 frames per world frame (320 per second), but unjoined bot movement
offers it more: 96 bots at 4 updates per second (384) in Acquisition, and
about 880 in Domination. With joining alone, the four frames per world frame
cannot get through the joiner's creation snapshots, which do not join.

## Status

Opt-in. Pair it with `--coalesce-character-replication`
([character-replication-coalescing.md](character-replication-coalescing.md))
for 128- and 256-character worlds on the test server first.
