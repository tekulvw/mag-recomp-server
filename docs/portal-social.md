# Portal friends, requests, blocks and presence

Retail MAG built its in-game friends list from the console network's friend list and presence:
the game published a small presence blob for its player and turned friends' presence into
online, offline and removal events. The replacement runtime has no such network, so the accounts
portal keeps the friend graph and relays presence. A client-side provider in the game runtime
uploads its player's presence and polls friends' presence, turning the answers into the events
the game expects. This page is the server contract. The portal's [README](../portal/README.md)
covers the rest of the portal.

The game's online ID is the portal player name. Presence blobs are opaque: the portal stores and
returns them byte for byte and never parses them.

## Rules

- **Friendships are mutual.** One player sends a request; it becomes a friendship when the other
  accepts it or sends a request back (crossing requests become friends at once).
- **Presence is shown to mutual friends only**, whatever their faction (the game enforces its
  own party and faction rules). Players with a pending request, strangers and blocked players
  never see it.
- **Presence lasts 60 seconds.** Only the latest blob is kept. A friend is `online` while their
  latest blob is under 60 seconds old; `state: "offline"` deletes it at once, and the portal's
  cleanup task purges stale rows.
- **Blocking** ends any friendship and drops every request between the two players. The blocked
  player is not told (see [Blocks and privacy](#blocks-and-privacy)).
- **Limits:** 200 friends, 50 outgoing requests waiting, 1,000 blocked players per account.
- **Who may call:** any account (person or service) with a player name that is not banned.
  Deleting an account deletes its friendships, requests, blocks and presence.

## API

Every call takes `Authorization: Bearer <device token>` (the launcher's `mgd_` token, as for
tickets) and answers JSON. Names in paths and bodies match player names case-insensitively;
responses use the canonical spelling.

### Errors common to every call

| Status | `error` | When |
| --- | --- | --- |
| 401 | `invalid_token` | No token, or not a current device token. |
| 403 | `banned` | The account is banned. |
| 409 | `no_player_name` | The account has no player name yet. |
| 429 | `rate_limited` | Over the account's per-minute allowance; `Retry-After: 60`. |
| 404 | `unknown_name` | The named player does not exist (calls that name a player). |
| 400 | `self` | The call names the caller (calls that name another player). |
| 400 | `invalid_request` | The body is not the expected JSON. |
| 500 | `server_error` | The portal failed; retry later. |

Error bodies are `{"error": "<code>"}`.

### Read the lists — `GET /api/v1/social`

```json
{
  "self": "Alpha",
  "friends": [
    { "name": "Bravo", "online": true,
      "presence": { "data_b64": "AAEC/w==", "updated_at": "2026-11-23T18:00:05Z" } },
    { "name": "Charlie", "online": false, "presence": null }
  ],
  "incoming": ["Delta"],
  "outgoing": ["Echo"],
  "blocked": ["Foxtrot"],
  "poll_seconds": 15
}
```

`presence` is present exactly when `online` is true; `data_b64` is standard base64 with padding
and `updated_at` RFC 3339 UTC. Friends and blocked players are in name order, requests oldest
first. `poll_seconds` is how often clients should poll; the server may change it, so clients
should read it every time. `incoming` leaves out requests from players the caller blocks.

### Publish presence — `PUT /api/v1/social/presence`

```json
{ "data_b64": "<base64>", "state": "online" }
```

`state` is `online` (the default when absent) or `offline`. Online needs `data_b64`: standard
base64 of at most 1,024 bytes (empty is allowed). Offline ignores `data_b64` and deletes the
stored presence. Answers `204`. `413 too_large` when the blob exceeds 1,024 bytes or the body
4 KiB; `400 invalid_request` for bad base64 or an unknown state. Clients should send presence
whenever it changes and at least every 30 seconds while playing, and `offline` when the game
leaves the network.

### Send a request — `POST /api/v1/social/requests`

Body `{"name": "Bravo"}`. Answers `201` with `{"status": "requested", "name": "Bravo"}`, or
`{"status": "accepted", "name": "Bravo"}` when Bravo had already asked the caller (they are now
friends). Sending a request that is already waiting answers `201 requested` again.

| Status | `error` | When |
| --- | --- | --- |
| 409 | `already_friends` | They are already friends. |
| 403 | `blocked` | The caller blocks that player (unblock first). |
| 409 | `friend_limit` | The caller has 200 friends, or (when the request would be accepted) either side has. |
| 409 | `request_limit` | The caller has 50 requests waiting. |

### Answer, cancel, remove

| Call | Answer |
| --- | --- |
| `POST /api/v1/social/requests/{name}/accept` | `204`; `404 no_request` when nothing from them is waiting; `409 friend_limit` when either side has 200 friends. |
| `POST /api/v1/social/requests/{name}/decline` | `204`, also when nothing was waiting. |
| `DELETE /api/v1/social/requests/{name}` | Cancels the caller's own request: `204`, also when nothing was waiting. |
| `DELETE /api/v1/social/friends/{name}` | Ends the friendship for both: `204`, also when they were not friends. |

Clients learn of removals and new friends from the next `GET /api/v1/social`: a name that leaves
`friends` is a removal.

### Block — `PUT /api/v1/social/blocks/{name}`, unblock — `DELETE /api/v1/social/blocks/{name}`

Both answer `204` and are idempotent. Blocking also ends the friendship and drops requests both
ways. `409 block_limit` past 1,000 blocked players.

### Look up a name — `GET /api/v1/social/lookup?name=Bravo`

`200 {"name": "Bravo", "exists": true}` for an exact (case-insensitive) match, otherwise
`404 unknown_name`. There is no prefix or partial search.

### Rate limits

Per account, per minute: 60 presence updates, 30 changes (requests, answers, removals, blocks and
unblocks, shared with the Friends page), 120 list reads (shared with the Friends page) and 30
lookups. Over a limit: `429 rate_limited` with `Retry-After: 60`.

## Blocks and privacy

A blocked player cannot detect the block. Their requests to the blocker answer `201 requested`
like any other and show in their own `outgoing` list, but the blocker never sees them: they are
left out of `incoming` and cannot be accepted. The blocker gets `403 blocked` for their own
requests, since they already know whom they block. Unblocking reveals a request the blocked
player sent meanwhile, as if it had just arrived. A `403` for a request to someone who blocks you,
or a `404`, would have told the requester; this is why the contract's `403 blocked` applies only
to the caller's own blocks.

## Friends page

Signed-in players with a name manage the same lists at `/friends`: send requests by name, accept
or decline incoming requests, cancel their own, remove friends, block (from a list or by name)
and unblock. Each friend shows online or offline; presence blobs are never shown. Forms carry the
session's CSRF token and share the per-account limits above. Banned players get `403`.

Staff see each account's friend, request and block counts on its admin page.

## In-game Community screen

The game's COMMUNITY screen stays hidden until the player's clan list arrives, so the Friends tab
depends on two Medius (MLS) answers besides the portal's presence:

- **Plugin id.** The retail client sends its MLS plugin messages with the plugin id of the last
  plugin message it received, and its clan requests stay unsent while that id is 0. Retail MLS
  therefore sends every plugin message with plugin id 1 and accepts 0 (the client's hello) or 1
  from the client. The prototype keeps 0.
- **Clan list.** Clans are not implemented, so every account belongs to none. Clan messages use
  MAG plugin type `0x01FC` in both directions: the big-endian MAG type, then a clan body whose
  first byte is the clan message type and whose fields are little-endian. The clan list request
  is type 2 with a 32-bit request id and the player's 32-bit account id. MLS answers it with type
  2, the same request id, status 10 ("no results", which the game counts as success) and a final
  byte of 1 marking the last packet. A request naming another account gets the same answer and is
  recorded in the MLS log. Other clan types are logged with their first bytes and not answered.

## Storage

Migration `0005_social.sql`: `friendships` (one row per pair, lower account ID first),
`friend_requests`, `blocks` and `presence` (`account_id`, `data` of at most 1,024 bytes,
`updated_at`). Every row cascades with either account. Changes that read before they write take
per-account advisory locks in account order, so racing requests between the same players and the
list limits stay consistent.
