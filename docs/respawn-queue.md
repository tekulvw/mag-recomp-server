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
- The world clock does this rejoining, at the first world tick that sees the
  character dead. It does not wait for the client's next update: a dead client
  reports only about once a second, so a wave that fell between the death and
  that report used to go by, and the player waited a whole extra period after
  the countdown reached zero. The client's first update after the wave starts
  the new life. Embeddings without world ticks still rejoin on that update.
- A character that bled out on its own (the bleeding-out report described in
  [incapacitation.md](incapacitation.md)) is queued for the first wave after
  its client's respawn screen opens: 2.5 s after the report (the retail
  `fRespawnTime`), or as soon as the client confirms if that comes first.
  The client is still playing its bleed-out until then. A life sent earlier
  reaches it in that state, and the client never leaves the state the new
  life starts in: a parachute drop falls straight down and keeps its
  parachute after landing, and the character can no longer enter a vehicle.
- A downed character gives up its downed life when it confirms: the
  confirmation clears the downed state (the death keeps the attribution of the
  hit that downed it, as a bleed-out does, and the character can no longer be
  revived) and joins the next wave in the same step. Retail's confirmation
  handler does the same: it clears the incapacitated condition before its
  dead check. The retail client sends this confirmation by itself when its
  respawn screen opens, about 2.5 s after the player presses Bleed out, and
  the bleed-out press has normally given the life up already (see
  [incapacitation.md](incapacitation.md)).
- No reply is sent to a confirmation. The retail server sends none, and the
  client only logs one; its deploy button toggles between confirm and cancel
  on each press by itself.
- `CancelRespawn` (`0x0213`) takes a dead character out of the queue, including
  one that gave up its downed life with a confirmation. It then
  stays out until the player confirms again. The next death clears this hold.
- The queue is not joined once the match has ended.

Server-owned bots keep their own respawn scheduling and are not affected.

The world log records `automatic_respawn_queued` (reservation, team and
deadline) when the world clock queues a dead character. The object-update audit
records `automatic_respawn_deadline_ms` when an owner update did it instead.
