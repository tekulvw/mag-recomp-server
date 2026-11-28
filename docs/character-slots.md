# Character slots

Each account holds up to three characters, in slots 0 to 2. Players create a character in any
free slot, choose its faction freely, pick one at the character select screen, and can delete
one there. Before this change an account held one character.

## What retail did

- **Three slots.** Retail MAG gave every account one character slot. From 31 August 2010,
  two more appeared on the character login screen, each a $0.99 30-day PlayStation Store
  subscription. A character in an unpaid slot went dormant, not deleted. This server gives all
  three free.
- **The client takes the count from the server.** The character list (`0x0237`) carries the
  number of allowed characters (a byte the frontend reads as `NumAllowedCharacters`) before the
  summaries. The client's built-in offline server lists one character and two cleared summaries,
  which is the same three-slot layout.
- **No slot field on the wire.** A slot is a position in that list. Creation (`0x023A`) has no
  slot field. The client names a character after the slot the player picked: the stored name is
  the online ID, `!`, and the slot (`alpha!0`). Other players never see that name; they see the
  online ID from the character's NP identity.
- **Any faction in any slot.** Nothing in the client or the store ties an account's characters to
  one faction; the extra slots were sold as a way to play another faction without leaving one.
  Veteran and faction respec (`0x025A`/`0x025B`, level-60 rules) are separate, per-character
  features that this server does not implement yet.
- **Delete exists.** The character screen's delete command sends `0x023C` over MAS, after
  authentication, exactly like creation: the common header, the summary's name (uint16 length and
  bytes) and its id (uint32). The answer, `0x023D`, is the common header and one success bit. Both
  layouts are the same in the prototype and the retail v1.0 client. Retail's server deleted the
  character.
- **The client stays connected after a delete.** Unlike creation, which goes straight on to the
  account login, a delete leaves the client on character select with its MAS channel open. It
  sends a keepalive (`0x0003`, a bare message type) every 10 s and drops the connection ("The
  connection to the game universe was lost (2:4)") about 10 s after one goes unanswered. The
  server echoes each keepalive and keeps the session signed in, so the player can create another
  character and log in on the same channel.

## What this server does

- **The list** allows three characters. Position N is slot N: a slot freed below an occupied one
  is sent as a cleared summary (id 0, which the client treats as empty), so the characters above
  it keep their places. Trailing empty slots are left out, so an account whose characters fill its
  first slots gets exactly the list it always had.
- **Creation** uses the slot the name's `!N` suffix names when that slot is free, otherwise the
  lowest free slot. A fourth character is refused with the client's generic error
  (`kNetError`, 1); retail has no result code for a full account. Names stay unique across all
  accounts, ignoring case, and an exact retry of a creation still succeeds.
- **Login** binds the session to the character the player picked (the client logs in with the
  listed name). Rewards, progression, loadouts, skills, nominations, leadership and Training
  completion all belong to that character; each character progresses on its own.
- **Voice sanctions** belong to the account, as before: a mute or ban covers every character, and
  it outlives a deleted or purged character.
- **Player reports** on the portal name what other players see, the account's online ID; the
  portal records the account.

## Deletion

Deleting is a **soft delete on the server**, a deliberate change from retail's hard delete: a
misclick or a stolen account should not destroy a high-level character for good.

- The client sees retail behaviour: `0x023D` reports success, the character leaves the list, and
  its slot is free at once.
- The server keeps the row, its kits, progression and reward history, marked with the time of the
  deletion. Rewards still in flight from its last match land on it.
- The character's stored name is moved aside to `<name>~<id>` (shortened to fit 32 characters),
  and the original is kept in the row. The client names characters after their slot, so this
  lets the player create a new character in the same slot, under the same name, straight away.
- An operator brings a deleted character back with
  `mag-admin character restore <account> <name>` (or the admin API). It goes into the account's
  lowest free slot, with everything it had. It gets its old name back when no other character
  holds it now; otherwise it keeps the moved-aside name, which the client logs in with just as
  well. Restore is refused while all three slots hold live characters.
- `mag-admin character purge <account> <name>` removes a deleted character for good, with its
  kits, match rows, receipts and match history. Only deleted characters can be purged.

With the PostgreSQL store every deletion, restore and purge is journalled in `mag.admin_changes`
with the character row before and after it.

## Storage

- **JSON store.** Characters carry `slot` (written only when not 0), `deletedAt` (Unix seconds)
  and `originalName` (deleted characters only). A store with one character per account is
  unchanged on disk. The reader refuses two live characters in one slot, a slot past 2, and an
  original name on a live character.
- **PostgreSQL**, migration `0003_character_slots.sql`:
  - drops the unique constraint on `mag.characters.account`;
  - adds `slot` (0 to 2, default 0, so every existing character becomes slot 0), `deleted_at`
    and `original_name`;
  - adds a unique index on `(account, slot)` over live characters, and an index on `account`;
  - drops the foreign key from `mag.voice_sanctions` to the account's character (an account check
    replaces it);
  - adds `character_id` to `mag.admin_changes`.

  Creation, deletion and restore take a per-account advisory lock, so concurrent requests for one
  account cannot both claim a slot.

The migration is one-way: an older Medius refuses a schema newer than its own. Rolling back means
an `export` to JSON (see [admin-cli.md](admin-cli.md#postgresql-character-store)), and an older
JSON reader refuses a store in which an account has more than one character.

## Rollout

1. Take a `pg_dump` of the character database before rolling out, as for 0.14.0.
2. Roll Medius. Main Medius applies migration 3 at startup. MAPS and `mag-admin` only check the
   schema: run them from the same build, after main Medius has migrated.
3. Check `mag-admin character-store status`: schema `3 of 3`, the same character count as
   before. The contents digest is unchanged for a store with one character per account.
