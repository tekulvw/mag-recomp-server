# Character replication coalescing

`--coalesce-character-replication` is an opt-in worker flag. It is off by
default, and without it every queued frame goes out exactly as before.

## What it changes

Every bot step queues its own Character delta for each ready recipient. A
step can carry movement, aim and ammunition, or landing damage. Each delta is
a complete update: a 147- or 148-bit header, one Character row and the list
terminator. A typical bot movement delta is 31 bytes, and 19 of them are
header. In a 128- or 256-character world, a recipient receives dozens of
these per tick. Each one uses a message slot and a sequence number, and
the bridge reply budget is shared by every recipient (128 messages and
24,000 payload bytes per reply).

With the flag, the shared drain joins consecutive queued Character deltas of
one recipient into one update. The joined update has one header with the
recipient's next sequence, every row in queue order, and one terminator. A
native update already holds a list of objects for each type its mask selects,
so the joined update uses no new wire shape. It does put several Characters in
one update, which no server path sent before this flag.

A frame joins only when all of these hold:

- it is a server update (no NAK sequence, client index 0) whose only object
  type is Character;
- it holds exactly one row, and every present field is one the server's delta
  encoder writes with a fixed width: state, position, pitch, yaw, damage,
  primary item, secondary item, primary equipped;
- nothing but the list terminator and zero padding follows that row;
- its header timestamp equals the first joined frame's, so rows of one world
  tick join and rows of different ticks do not;
- its Character is not already in the update, and is not the recipient's own
  Character (owner position corrections stay bound to their own sequence);
- it is not part of a hospital or smoke bootstrap;
- the joined update stays within 1,397 bytes and within the reply's remaining
  payload budget.

The first frame that fails a check ends the run and is sent as before, so the
recipient applies every field in the order it was queued. The joined update
is what the recipient's NAK history keeps and replays. A run of one frame is
sent exactly as without the flag.

## Measured effect

Headless sims on private Acquisition and Domination inputs with authored
capacity: 128 and 256 characters, 32 observers, 10 client events per second
per observer, 900 simulated seconds, statistics after 120 s, Apple M4 Pro,
`nice -n 15`, one sim at a time. Byte counts are plugin payload.

| World | Flag | Per-recipient messages/s | Per-recipient KB/s | Drain mean (ms) | Tick mean / p99 (ms) | Evictions |
|---|---|---|---|---|---|---|
| Acquisition 128 (96 bots) | off | 270 | 8.3 | 0.29 | 2.08 / 5.47 | 0 |
| Acquisition 128 (96 bots) | on | 25 | 3.5 | 0.23 | 1.86 / 5.71 | 0 |
| Domination 256 (224 bots) | off | 498 | 15.1 | 0.53 | 7.71 / 12.10 | 0 |
| Domination 256 (224 bots) | on | 30 | 6.4 | 0.43 | 6.00 / 10.36 | 0 |

Joining does not help a recipient whose queue is led by frames that do not
join, such as the creation snapshots a joiner receives for every bot. With no
client events (world frames alone drain the queues), all 32 observers were
still evicted as `replication_backlogged` about 2 s after the bots spawned,
with or without the flag. The shared reply budget, four messages per
recipient per world frame at 32 recipients, is the limit there.

## Status

The retail client has not yet been shown a joined update. Turn the flag on
only for a test-server check, with packet receipts, before any live use.
