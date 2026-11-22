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
