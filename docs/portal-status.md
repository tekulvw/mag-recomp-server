# Server status on the portal

The portal's home page (`/`) shows whether each game server is up, for anyone, signed in or not.
The same status is at `/status` and as JSON at `/api/v1/status`. None of them needs a session
or touches an admin route. Both pages update the status block in place every refresh interval
(see [Live updates](#live-updates)); without scripts they show it as it was when loaded.

## What it shows

Per server:

- **State.** `up` when the game server answered recently and every world admits players,
  `degraded` when some do, `down` when none do or the server has not answered for three refresh
  intervals (plus 5 seconds). Down shows when it was last seen. `unknown` until the first poll.
- **Per world:** open or closed (`open`, `draining`, `faulted` or `unavailable` from Medius; see
  [the admin API](admin-cli.md#admin-api)), the mode, and the match in progress if any: sides,
  map profile, humans against the world's human limit, and bots. Humans waiting for that world.
- **Totals:** players online with the server-wide human limit, humans waiting for a match
  (counts only), game (DME) ports in use against the cap, uptime since Medius last started, and
  the Medius and gameplay versions.

Human limits are worded so a world's size is never read as a human cap. Every human holds one
of the server's DME ports, so the port capacity (32 on live) is the most humans the whole server
holds at once, across all modes, and no world holds more. A world's limit is its configured row
and profile cap within that: 32 for live's 64-Character Sabotage row (whose seats may all be
human) as for its 128- and 256-Character worlds (capped at 32 humans). Bots fill the rest of the
world. So a world reads "Humans 3 / 32 · bots fill the rest (256-player match)", or "Humans 3 /
16 (16-player match)" without bot fill, and the totals "Players online 3 · up to 32 humans at
once across all modes". The JSON has the same text (`humans_text`, `players_text`) beside the
numbers: `human_limit`, `world_size` and `bot_fill`. `human_capacity` is the row and profile cap
alone, kept for older readers. With a game server that predates `human_limit`, the portal bounds
`human_capacity` by the port cap itself.

The page and the JSON carry counts only. The portal copies named fields from the game server's
`GET /admin/v1/status` (which itself names no account) into its own view; it never shows account
or player names, and never the game servers' addresses.

Above the status, staff can show a **maintenance or incident banner**: admins set its text and
an optional "restart expected at" time (UTC) on the admin panel's Status page (`/admin/status`),
or remove it. Each change goes to the audit log. The banner shows on the home page, `/status`
and in the JSON. Moderators see the admin Status page, including staff-only servers, but cannot
change the banner.

## How it works

A background task polls each configured server's admin API (`GET /admin/v1/status`, signed like
every admin API request, actor `portal status page`) every `PORTAL_STATUS_REFRESH_SECONDS`
(default 15) and reloads the banner from the database. Pages and the JSON read only that
in-memory cache, so public traffic never reaches a game server or, for anonymous visitors, the
database. One failed poll between good ones does not mark a server down. The portal logs a
server going down or coming back, not every failed poll. A game server too old to report its
status shows as down with "This game server version does not report its status."

## Live updates

The home page and `/status` load `/status.js`, a small same-origin script (the site's
Content-Security-Policy allows scripts and fetches from its own origin only, and no inline
script). While the tab is visible it fetches `/api/v1/status` every refresh interval, which the
portal serves from its cache, and rebuilds the status block, its heading and the banner in
place. It builds elements with `textContent` only and never parses anything from the answer as
HTML. It stops while the tab is hidden and fetches at once when the tab is shown again. A line
under the block says how long ago the data was updated; when a fetch fails it keeps the last
data, dims it and says it could not refresh. Without scripts, the server-rendered block stays
as loaded.

## Configuration

| Variable | Meaning |
| --- | --- |
| `PORTAL_STATUS_SERVERS` | Comma-separated server ids, in display order, such as `live,modes`. Ids are 1 to 32 lowercase letters, digits or `-`. Unset: `live` when `PORTAL_GAME_ADMIN_URL` is set, otherwise no status. |
| `PORTAL_STATUS_<ID>_ADMIN_URL`, `PORTAL_STATUS_<ID>_ADMIN_SECRET_FILE` | That server's admin API origin and shared-secret file, both or neither (`<ID>` is the id in upper case, `-` as `_`). `live` without them uses `PORTAL_GAME_ADMIN_URL` and `PORTAL_GAME_ADMIN_SECRET_FILE`. |
| `PORTAL_STATUS_<ID>_NAME` | Display name (default `Live` for `live`, else the id). |
| `PORTAL_STATUS_<ID>_HOST` | Host name shown under the name, such as `play.magtest.online`. |
| `PORTAL_STATUS_<ID>_STAFF_ONLY` | `true` shows the server only on the admin panel's Status page (for a test server). Default `false`. |
| `PORTAL_STATUS_REFRESH_SECONDS` | Seconds between polls, 5 to 300 (default 15). |

Each game server's admin API must allow the portal in its NetworkPolicy, as for the admin
panel's Game server section. The status endpoint is read-only, but it shares the admin API's
secret, so the same rule applies: never route the admin API through a Gateway.
