# Portal player reports

Players report other players' conduct on the accounts portal, and moderators work through the
reports in the admin panel. This is a moderator queue only: nothing is attached to a report
automatically, nothing happens to the reported player until a moderator acts, and there is no
in-game route yet. The portal's [README](../portal/README.md) covers the rest of the portal.

## Players

A signed-in player with a player name and no ban files a report at `/reports/new`, linked from
the account page (not from the public pages).

- **Who:** a character, by name. The form suggests names as the player types (from
  `/reports/players`, see below); without JavaScript, its search box lists matching names to
  pick. Any portal player name, reserved name (an existing player's, or a deleted account's) or
  character the game server lists can be reported. A name only the game server knows cannot be
  checked while the game server is unreachable, and the form says so.
- **Reason (one):** cheating, harassment or voice abuse, team-killing or griefing, offensive
  name, or other.
- **What happened:** required, at most 1,000 characters. Line breaks are kept; other control
  characters and direction overrides are dropped.
- **About when:** optional, in the player's local time. The shared time script (`/time.js`) names
  the zone beside the field and sends the browser's offset, and the portal stores the time in UTC
  (without the script the field says UTC and the time is taken as UTC). It
  must be within the last year and not more than a day ahead.

The rules:

- **Not yourself**, in any spelling.
- **One open report per player and target.** A second report about the same character is
  refused (`409`) while the first is open, and does not count against the allowance. Once a
  moderator has reviewed it, the player can report the character again.
- **A daily allowance:** `PORTAL_PLAYER_REPORTS_PER_DAY` (default 5) reports per account in any
  24 hours. More are refused (`429`).

`/reports` lists the player's own reports with their status: **open** or **reviewed**. Players
never see the moderator's note, decision or name, and the reported player sees nothing at all.

### Name suggestions

`GET /reports/players?q=<text>` answers `{"players": ["Bravo", "brawler"]}`: up to 10 names
containing the text (any case), names starting with it first, never the asking player's own.
It takes the web session cookie, answers `401` without one, and allows 120 lookups a minute per
account (`429` after). It needs at least 2 characters of a possible name; anything else gives an
empty list. Names are spelled as their portal account or reserved name spells them, else as the
game server's lowercase key.

The game server's character list comes from its admin API (`GET /admin/v1/accounts`, with the
actor `portal player reports (lookup)`), fetched at most once a minute and reused; after a
failure the portal waits 15 seconds before asking again. Portal player names and reserved names
are always searched, so suggestions keep working when the game server is down.

Answers carry names only: no account numbers, emails or sign-in identities.

## Moderators

Moderators and above see **Player reports** in the admin panel (`/admin/player-reports`); the
overview shows the number of open reports. Everyone else gets 404 there.

- **The queue** lists reports newest first, filtered by status (open by default, actioned,
  dismissed or all) and reason, with each target's count of other reports (and how many are
  open) and the reporter.
- **A report** shows the reporter (linked to their user page), the description, the approximate
  time, the other reports about the same character, and links to the character's game server
  page (voice mute and ban, levels) and the target's user page (account ban). Penalties stay with
  those tools.
- **Review:** mark an open report **actioned** or **dismissed** with a note (required, at most
  1,000 characters, staff only), or **reopen** a reviewed one (note optional). Reopening is
  refused while the reporter has a newer open report about the same character.
- **Staff targets:** a report about a staff member can only be reviewed by someone ranked above
  them, so nobody reviews reports about themselves or a peer.
- **Audit:** every status change is written to the audit log in the same transaction as the
  change, as `action player report`, `dismiss player report` or `reopen player report`, with
  the target `report <id> on <name>` and the note as the detail.

## Storage and privacy

Reports live in the portal's database (`player_reports`, migration `0008`), indexed for the
queue (status, newest first), a target's other reports, a reporter's list and allowance, and a
unique index that keeps one open report per reporter and target.

- The reporter's identity is visible to staff only.
- Deleting the reporter's account keeps their reports for the moderators, without the account
  (`reporter_account` becomes NULL; pages show "deleted account").
- Deleting the reported player's account keeps the reports on the character, whose name stays
  reserved (`target_account` becomes NULL).
- The privacy page says what a report keeps.

## Not done yet

- **Players from your recent matches.** The game server's character store records each
  character's matches (`match_progress` and `match_history`, keyed `world-<world>-<match>`), so
  recent co-players could be listed without gameplay changes. It needs a new read-only admin
  endpoint and a store query on both character store backends (file and PostgreSQL), and would
  list only players who earned a reward in the match. Until then players type or search the
  name.
- No in-game reporting, automatic evidence or automatic penalties.
