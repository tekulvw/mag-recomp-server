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
| 26 | 16 | nonce | random; single use |
| 42 | 1 | `A` | 1 to 63 |
| 43 | `A` | audience | the game server's host name, lowercase ASCII letters, digits, `.` and `-` |
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
   lifetime (`expires at − issued at`) is at most 3600 seconds.
4. The audience equals the verifier's own configured host name.
5. The account ID is not 0 and the name follows the rule above.
6. The nonce has not been accepted before. Remember accepted nonces until their ticket expires.

The player name keeps the case the player chose. Servers that key accounts by lowercase name
lowercase it after verification.

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
  authenticate messages are ignored until it reconnects. A rejected ticket gets no reply, as a
  rejected development name does.
- Accepted nonces are kept in memory until their ticket expires, at most 65536 at a time; when
  that many live nonces are held, new tickets are refused rather than old nonces forgotten. Each
  Medius process has its own cache, and MAPS and MAS run in separate processes, so each service
  accepts a ticket once. A restart forgets the cache; the short ticket lifetime bounds that window.
- The session's account is the ticket's player name in lowercase, which is how local characters
  are keyed, so a player who claims their existing name keeps their characters. The portal
  account ID stays on the session (`PortalAccountId`) for later use. A character's NP ID must
  still be all zero or carry an online ID equal to that name, ignoring case.
- Logs carry only the reason class (`ticket=accepted`, `ticket=expired`, `ticket=replayed`, …)
  in the session state, and the retail no-reply log prints `payload=redacted` for NP
  authenticate messages. Ticket bytes are never logged.

Rolling out:

1. Fetch the keys from the portal's `GET /api/v1/ticket-keys` and set the audience to the host
   name players connect to. Keys are public; they need no Secret.
2. Make sure players have claimed their names on the portal first. Operators import existing
   names as reserved and assign them to their owners, so nobody else can take them.
3. Switch to `required` and restart Medius. Clients that still send a plain name can no longer
   sign in. Switching back to `off` restores the development sign-in.
4. To rotate, publish the new key on the portal, add it next to the old one here, restart, let
   the portal sign with it, then remove the old key after the last old tickets have expired
   (at most an hour).
