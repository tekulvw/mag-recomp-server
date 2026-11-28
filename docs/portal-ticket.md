# Portal game ticket, version 1

The portal (`portal/`) issues a game ticket to a linked launcher. The game sends the ticket
unchanged in its NP authenticate message, and the game server verifies it with the portal's
public key. The ticket carries no Google data.

## Layout

All integers are unsigned and big-endian. `A` is the audience length and `N` the name length.

| Offset | Size | Field | Rule |
| --- | --- | --- | --- |
| 0 | 4 | magic | ASCII `MAGT` |
| 4 | 1 | version | `1` |
| 5 | 1 | key ID | selects the verifying key; lets the portal rotate keys |
| 6 | 8 | issued at | Unix seconds |
| 14 | 8 | expires at | Unix seconds, greater than issued at |
| 22 | 4 | account ID | portal account ID, never 0 |
| 26 | 16 | nonce | random; limits reuse (rule 6) |
| 42 | 1 | `A` | 1 to 63 |
| 43 | `A` | audience | the host name of the game server the ticket is for, lowercase ASCII letters, digits, `.` and `-` |
| 43+`A` | 1 | `N` | 3 to 15 |
| 44+`A` | `N` | player name | ASCII letters, digits, `-` and `_`, starting with a letter |
| 44+`A`+`N` | 64 | signature | ECDSA P-256 over SHA-256 of bytes `0 … 43+A+N`, as `r ‖ s` (32 bytes each) |

The total length is exactly `108 + A + N` bytes: 142 for `play.magtest.online` and a
15-character name.

## Verifying

A verifier accepts a ticket only when all of these hold, and otherwise rejects it without
saying which check failed to the client:

1. The length is exactly `108 + A + N`, the magic and version match, and `A` and `N` are in range.
2. The key ID names a configured key, and the signature verifies with it. Signatures in either
   `s` form are valid; the portal emits low `s`.
3. `issued at` is no more than 60 seconds in the future, `expires at` is in the future, and the
   lifetime (`expires at − issued at`) is at most 43200 seconds (12 hours).
4. The audience equals the verifier's own configured host name. One portal can issue tickets for
   several game servers (such as `play.magtest.online` and a LAN test server,
   `test.magtest.online`); the audience keeps a ticket for one from being accepted by another.
5. The account ID is not 0 and the name follows the rule above.
6. The nonce has been accepted fewer times than the verifier's reuse limit, and remembered until
   its ticket expires. The game reuses one ticket for several sign-ins (MAPS, MAS and statistics
   login) and for every reconnect until it fetches a new one, so a verifier may allow a ticket a
   fixed number of uses instead of one. Medius allows 32 per service and never lets a ticket open
   a second live session for an account that is already signed in there.

The player name keeps the case the player chose. Servers that key accounts by lowercase name
lowercase it after verification.

## Lifetime

The portal signs tickets for `PORTAL_TICKET_LIFETIME_SECONDS`: 600 by default, at most 43200
(12 hours). The live portal uses 12 hours. A launcher fetches one ticket when it starts the game,
and the game keeps sending those bytes, including when it reconnects after the server dropped it;
a ticket that expires mid-session turns that reconnect into a refused sign-in.

The trade-off: a ticket is a bearer credential until it expires, so a leaked one can sign in as
its account for up to 12 hours. Two things limit that. A ticket never opens a second live session
for an account that is already signed in on the same service, and each service accepts it at most
32 times. A portal ban (`ban <name>`) stops new tickets at once but does not revoke tickets already
issued: they keep working until they expire, so a ban takes full effect within the lifetime.

## Audiences

The portal issues tickets for its default audience, `PORTAL_TICKET_AUDIENCE`, and for any host
listed in `PORTAL_TICKET_EXTRA_AUDIENCES`. A launcher picks one with the optional body
`{"audience": "<host>"}` on `POST /api/v1/ticket` (lowercased by the portal); without a body, or
without the field, it gets the default. An unlisted host gets `400 {"error":"unknown_audience"}`.
The response's `audience` field names the audience written into the ticket. Every game server
verifies with the same published keys and its own host name, so the same launcher token serves
each of them, and a ticket for one cannot be used on another.

## Keys

Verifying keys are P-256 public keys as DER SubjectPublicKeyInfo. The portal serves them at
`GET /api/v1/ticket-keys`:

```json
{"keys":[{"id":1,"alg":"ES256","spki":"<base64 DER>"}]}
```

Servers should take the keys from configuration rather than fetching them at sign-in, so
sign-in keeps working while the portal is down.

## Test vectors

`portal/tests/ticket-vectors.json` lists tickets as their field values, the test key, the
expected signature and the SHA-256 of the full ticket. A verifier test rebuilds each ticket
from its fields, appends the signature, checks the hash, and verifies it at the stated time.
The test key exists only for these vectors.

## Medius verification (operators)

Medius verifies tickets on both of its services, MAPS and MAS, when `MAG_PORTAL_TICKETS=required`.
The switch is read once at startup:

| Variable | Meaning |
| --- | --- |
| `MAG_PORTAL_TICKETS` | `off` (default, also when unset): the development name sign-in is unchanged and the other two variables are ignored. `required`: only valid portal tickets sign in; the plain development name is refused even when `EnableMagDevelopmentAuth` is on. |
| `MAG_PORTAL_TICKET_KEYS` | Comma-separated `id:base64` entries: the key ID (0 to 255) and the DER SubjectPublicKeyInfo of a P-256 key, as `GET /api/v1/ticket-keys` serves them. List several during rotation. |
| `MAG_PORTAL_TICKET_AUDIENCE` | This server's host name, exactly as the portal writes it into tickets. |

With `required`, Medius refuses to start when the keys or the audience are missing or invalid,
naming the variable. The chart sets all three from `portalTickets` (see its README).

What Medius does with a ticket:

- It reassembles the NP authenticate chunks of one session. Every chunk repeats the full size,
  the chunks add up to it exactly and the last one carries the last-message flag. The full size is
  at most 2048 bytes, a ticket takes at most 8 chunks, and bytes after a chunk's size must be zero.
  The RSA words must be zero, as for the development sign-in. Any inconsistency discards the
  partial ticket.
- It applies the six rules above. A session gets three failed attempts; after that its NP
  authenticate messages are ignored until it reconnects.
- A refused ticket gets a failure reply: the NP authenticate reply with its success bit clear,
  echoing the request's transaction. The client reads that bit, so it ends the sign-in at once
  instead of waiting for a reply until it gives up. The reply does not say which rule failed. No
  reply is sent for a chunk that is not the last of its ticket (the client is still sending; its
  last chunk is answered), or after the session's three failed attempts. With tickets off, a
  rejected development name still gets no reply.
- Reuse: the game sends one ticket to MAPS (retail message 0x0034) and the same cached bytes to
  MAS and the statistics login, and again whenever it reconnects, so strict single use would lock
  players out. Each Medius process counts the uses of every nonce and accepts a ticket at most 32
  times within its lifetime (`ticket=replayed` after that). MAPS and MAS run as separate
  processes, so the count is per service. The count only stops unbounded reuse; the one live
  session per account rule below is what keeps a shared ticket from signing in a second player.
- Nonces are kept in memory until their ticket expires, at most 65536 at a time per service (about
  3 MB). A launch fetches one ticket, so a few thousand players a day use a small part of that.
  One account may hold at most 128 unexpired nonces (a fresh ticket every six minutes for 12
  hours); past that its new tickets are refused (`ticket=replay_cache_full`) until its old ones
  expire, while other accounts are unaffected, so it would take 512 accounts to fill the cache.
  When it is full of unexpired nonces, new tickets are refused rather than old nonces forgotten.
  Expired nonces are swept at least once a minute. A restart forgets the counts.
- One live session per account: a ticket is refused (`ticket=account_signed_in`) while another
  session on the same service holds its account, signed in, connected and not idle past the
  application's client timeout. A disconnect, or that timeout, frees the account for the next
  use. A refused attempt spends none of the ticket's uses. Medius tracks this itself: the MAG
  plugin sessions do not create Medius `ClientObject` logins, so the stock "already logged in"
  check cannot see them.
- The session's account is the ticket's player name in lowercase, which is how local characters
  are keyed, so a player who claims their existing name keeps their characters. The portal
  account ID stays on the session (`PortalAccountId`) for later use. A character's NP ID must
  still be all zero or carry an online ID equal to that name, ignoring case.
- Logs carry the reason class (`ticket=accepted`, `ticket=expired`, `ticket=replayed`, …) in
  the session state, and the retail no-reply log prints `payload=redacted` for NP authenticate
  messages. Every refusal also logs a warning for staff, such as
  `MAG Maps app=20231 portal ticket refused: reason=expired account=alpha account_id=42
  issued_at=2026-11-23T08:00:00Z expires_at=2026-11-23T08:10:00Z age_s=7260`. The account and
  times appear only when the portal's signature verified; a forged or garbled ticket logs only
  its reason. Ticket bytes and nonces are never logged.

Rolling out:

1. Fetch the keys from the portal's `GET /api/v1/ticket-keys` and set the audience to the host
   name players connect to. Keys are public; they need no Secret.
2. Make sure players have claimed their names on the portal first. Operators import existing
   names as reserved and assign them to their owners, so nobody else can take them.
3. Switch to `required` and restart Medius. Clients that still send a plain name can no longer
   sign in. Switching back to `off` restores the development sign-in.
4. To rotate, publish the new key on the portal, add it next to the old one here, restart, let
   the portal sign with it, then remove the old key after the last old tickets have expired
   (at most the portal's ticket lifetime, 12 hours on live).

## Optional Sony console authentication

`MAG_PORTAL_TICKETS=dual` retains MAGT verification and adds explicitly configured Sony NP verification and account linking. Existing `off` and `required` behavior is unchanged. See [dual authentication](dual-authentication.md) for trust configuration, portal database migration, browser linking and the unchecked real-console acceptance gates.
