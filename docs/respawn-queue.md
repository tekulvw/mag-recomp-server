# Human respawn queue and spawn selection

Behaviour of the replacement server, described in our own words.

## Selection

- A player's spawn choice (`0x0219`) is stored per player. It can be changed
  at any time: alive, downed or dead. It takes effect when the next life starts.
- The choice is kept across deaths. It is dropped only when the player
  changes squad, or when the squad can no longer use that point. In both cases
  the player follows the squad's shared spawn again.
- With no personal choice, the squad's shared spawn applies. The squad leader
  sets it with `0x0218`.
- If neither point can be used, the life starts at the squad's first usable,
  unblocked point, in authored order. A vehicle or hospital point that cannot
  seat the life starts it on foot at the squad's alternate insertion
  (see [apc-insertion.md](apc-insertion.md)).

## Queue

- Each team spawns in fixed waves on its own clock: a constant period,
  optionally scaled by Company strategics. A queued character starts its next
  life on the first wave after it joined the queue, so the countdown a dead
  player sees is anywhere between zero and one period.
- The first life joins the queue when the client confirms (`0x0211`).
- After a death, the character rejoins the queue for the next wave by itself
  once it is dead (a downed character only after bleed-out, finishing damage
  or give-up). It needs no new selection and no new confirmation. A
  confirmation is still accepted and simply joins the same wave.
- A downed character cannot join the queue. The retail client sends its
  confirmation while the map is open, which may be while downed, and does not
  resend it after bleeding out. The server therefore does not wait for one.
- `CancelRespawn` (`0x0213`) takes a dead character out of the queue. It then
  stays out until the player confirms again. The next death clears this hold.
- The queue is not joined once the match has ended.

Server-owned bots keep their own respawn scheduling and are not affected.

The object-update audit records `automatic_respawn_deadline_ms` when a dead
character rejoined the queue without a confirmation.
