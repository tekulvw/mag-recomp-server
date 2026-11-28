# Skill respec and the respec point floor

A player can reset their character's skill tree from the skill screen: a **respec**. It refunds the
skill points spent on skills, so they can be spent again. Respecs cost **respec points**, a balance
each character holds. An operator can give every character a minimum balance with
`MAG_RESPEC_POINTS_FLOOR`.

## What retail did

- **Two fields in the character record.** The full record (`0x0239`, and `0x024F` on refresh)
  carries `RespecCount` (a byte: respecs taken) and `RespecPoints` (a uint32: the balance). The
  character-list summary has a separate `RespecAvailable` bit for a faction respec; this change
  leaves that bit and the record's faction-respec bit cleared.
- **The request names nothing.** The skill screen sends `0x0254` with no payload. Like a skill
  purchase (`0x0250`), it is a game message on the lobby (MLS) channel, with no MAS header. It has no
  character, fee or transaction id; the server answers for the session's character.
- **The answer.** `0x0255` is a success bit. On success the full retail record follows, and the
  client takes the record as its new character state; a failure carries no record.
- **The rules.** A respec costs the authored fee for the character's respec count. The skill table
  holds the fees as rows (`ReSpec`, one `Points` value per row); once the count passes the last row,
  that row's fee repeats. The fees rise with each respec. An exact balance is enough. A respec refunds
  every owned skill at its authored cost (unspent points are capped at 255) and clears those skills
  and the equipment they awarded. It rebuilds the five kits from the faction's authored defaults,
  grants the free starter (tier 0) skills again, counts the respec (capped at 255) and debits the
  fee.
- **Earning points.** The only grant proven in the retail client is a creation-time credit
  (`AdditionalRespecPoints` in the creation settings), and the retail settings set it to zero. How
  retail's servers credited points after matches is not known. This server credits none, so without
  the floor every balance stays zero and a respec is always refused.

## What this server does

- **`0x0254` is handled** for the retail application by `LocalCharacterStore.RespecSkillsForAccount`,
  in one store transaction, with the rules above. Every request is answered. A refusal
  (`insufficient-respec-points`, `respec-fees-unavailable` when the skill catalog has no fee rows,
  `character-not-owned`, ...) is the bare failure bit and changes nothing. The MLS log line names
  the outcome: for example `skill-respec-committed/fee=<fee>/refunded=<points>`.
- **Kits.** The rebuilt kits are stored, so they replace any saved kits. Kit positions do not move.
  When the selected kit is empty after the rebuild, kit 0 (which the authored defaults always fill)
  becomes the selection. A character already in a match gets the new kits through the same refresh
  as an Armory edit. Its in-match skill modifiers (Character field 20, when that is sent) are those
  it was admitted with. The respec takes effect for them at the next admission, as a skill purchase
  does.
- **The floor.** With `MAG_RESPEC_POINTS_FLOOR=N`, every character, existing or new and in every
  slot, has at least N respec points whenever its record is built: at login, on the character list
  and player cards, in the respec answer and in the admin tools. The floor is applied when the
  record is built and is never written to the store, so turning it on rewrites no row. A respec
  spends from the balance raised to the floor and stores what remains. The next record raises that
  remainder to the floor again, so the floor tops the balance up after every spend. A stored
  balance above the floor is kept and spent from as usual. Unset (the default) or empty leaves
  every balance as stored; `0` does the same. A value that is not a whole number from 0 to
  4294967295 stops Medius at startup, naming the variable. Set it in main Medius and MAPS, as for
  `MAG_STARTING_LEVEL`. mag-admin reads it too, so that it shows the balance players see.
- **A floor below the next fee.** Each respec raises the fee, so a floor pays only for respecs
  whose fee it covers. Once a character's next fee is above the floor, that character cannot respec
  again until its stored balance makes up the difference.

## Storage

- **JSON store.** Characters carry `respecCount` and `respecPoints`, written only once a character
  has respecced. A store without them is unchanged on disk and reads both as 0.
- **PostgreSQL**, migration `0004_skill_respec.sql`, adds the nullable columns `respec_count`
  (0 to 255) and `respec_points` (0 to 4294967295) to `mag.characters`. No row is rewritten, and null
  reads as 0. As with every migration, an older Medius, MAPS or mag-admin refuses the newer schema.
  Take a `pg_dump` first, roll main Medius and MAPS together, and run mag-admin from the same build.
  Rolling back means an `export` to JSON (see [admin-cli.md](admin-cli.md)).

## Admin view

`mag-admin character show` prints a `respec:` line (the balance players see, the stored balance
when the floor raises it, respecs taken and the next fee), and each slot's respec points in the
character list. The admin API adds `respec_points` and `respec_count` to every character in
`GET /admin/v1/accounts` and `GET /admin/v1/characters/{account}`. The selected character also
gets `stored_respec_points` and `next_respec_fee`. The portal's game-admin character page shows
them per slot.
