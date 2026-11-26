# mag-portal

The player accounts website for a MAG replacement server (magtest.online). Players sign
in with Google, claim a player name and link their launcher. The launcher trades its
device token for expiring **game tickets**, which the game server verifies with the
portal's public key. See the [design](../docs/superpowers/specs/2026-11-22-portal-design.md)
and the [ticket format](../docs/portal-ticket.md). Launchers also send performance and error
reports with their logs: see [launcher reports](../docs/portal-reports.md). The home page shows each game server's status
(counts only, cached and polled in the background; see [server status](../docs/portal-status.md)). Players keep a
friends list (requests, blocks) at `/friends`, and game clients relay presence through the
portal: see [friends and presence](../docs/portal-social.md).

The site is server-rendered HTML. Its only JavaScript, `static/status.js`, keeps the home and
status pages' server status current without a reload; every page works without it. It stores accounts in PostgreSQL and
never passes Google tokens on to the game or the game server.

## Build and test

Tests need a PostgreSQL server they can create throwaway databases on:

```sh
docker run -d --rm --name portal-pg -e POSTGRES_USER=portal -e POSTGRES_PASSWORD=portal \
  -p 127.0.0.1:55432:5432 postgres:17-alpine -c fsync=off -c synchronous_commit=off -c full_page_writes=off
export DATABASE_URL=postgres://portal:portal@127.0.0.1:55432/portal
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

`docker build --target check portal` runs the same checks with its own PostgreSQL inside the
build; CI runs them natively with a cached dependency build. `docker build portal` builds the image, which runs `mag-portal serve` by default,
listening on port 8080 as UID 10001. The build downloads the web fonts listed in `fonts.txt`
(Chakra Petch, Inter and JetBrains Mono, all SIL Open Font License 1.1) and checks each against its
SHA-256; their licenses go to `/usr/share/licenses/mag-recomp-server/fonts/`.
Sign-in starts are limited to 30 a minute per client address.

Turning durability off (`fsync=off` and the rest) suits a throwaway test server only; it keeps
the checkpoint each test's `DROP DATABASE` waits for from stalling on disk writes. Database tests use `#[sqlx::test]` for their
database but open their pool through `tests/support` (see there why) and end with
`test_db.close().await`.

The tests run a fake OpenID provider and never contact Google. `tests/ticket-vectors.json`
holds ticket test vectors for other verifiers; regenerate them with
`PORTAL_WRITE_VECTORS=1 cargo test --test ticket_vectors` after a deliberate format change.

## Setting up

1. **Google OAuth client.** In Google Cloud console, create an OAuth client of type *Web
   application*. Add the authorized redirect URI `https://<site>/auth/google/callback`.
   The consent screen needs the site's home page and its privacy page
   (`https://<site>/privacy`). The portal asks only for the `openid` and `email` scopes.
2. **Ticket signing key.** `mag-portal keygen --out ticket-key.pem` writes a P-256 key
   (mode 0600) and prints its public key. The game server is configured with that public
   key; `mag-portal public-key --key-file ticket-key.pem` prints it again.
3. **Database.** An empty PostgreSQL database. `mag-portal serve` applies migrations at
   start; set `PORTAL_SKIP_MIGRATIONS=true` and run `mag-portal migrate` to do it separately.

## Configuration

| Variable | Required | Meaning |
| --- | --- | --- |
| `DATABASE_URL` | yes | PostgreSQL connection URL. Add `sslmode=verify-full&sslrootcert=…` for TLS. |
| `PORTAL_PUBLIC_URL` | yes | The site's origin, such as `https://magtest.online` (https, or http://localhost for testing). |
| `PORTAL_GOOGLE_CLIENT_ID` | yes | OAuth client ID. |
| `PORTAL_GOOGLE_CLIENT_SECRET_FILE` | yes | File holding the OAuth client secret. |
| `PORTAL_TICKET_KEY_FILE` | yes | PKCS#8 PEM P-256 private key from `mag-portal keygen`. |
| `PORTAL_TICKET_AUDIENCE` | yes | The game server's host name, such as `play.magtest.online`. Tickets are valid only there. It is the default audience: launchers that name none get it. |
| `PORTAL_TICKET_EXTRA_AUDIENCES` | no | More game server host names, comma-separated, such as `test.magtest.online`, held to the same rule. A launcher may ask for a ticket for any of them (see "Get a ticket"). |
| `PORTAL_TICKET_KEY_ID` | no | Key ID written into tickets (default 1). Change it when rotating keys. |
| `PORTAL_TICKET_LIFETIME_SECONDS` | no | Ticket lifetime, 60 to 43200 (default 600). The live portal uses 43200 (12 hours) so a game can reconnect with its launch ticket; see [Lifetime](../docs/portal-ticket.md#lifetime) for the trade-off. |
| `PORTAL_FONT_DIR` | no | Folder of the web fonts listed in `fonts.txt`, served at `/fonts/`. The image sets it; without it pages use system fonts. |
| `PORTAL_OWNER_EMAILS` | no | Comma-separated Google emails with the owner role in the admin panel. Owners cannot be changed from the site. |
| `PORTAL_CLIENT_IP_HEADER` | no | Header the trusted proxy appends the client address to (`x-forwarded-for`); the last entry is used. Without it, rate limits use the connecting address, which behind a proxy is the proxy. |
| `PORTAL_GAME_ADMIN_URL` | no | Origin of the game server's internal admin API, such as `http://mag-medius-admin.mag.svc:10490` (http or https). Set it with `PORTAL_GAME_ADMIN_SECRET_FILE`, or neither; without them the admin panel's Game server section shows "not configured". |
| `PORTAL_GAME_ADMIN_SECRET_FILE` | with the URL | File holding the admin API's shared HMAC secret, at least 32 bytes (surrounding whitespace is trimmed). Never logged. |
| `PORTAL_STATUS_SERVERS` | no | Game servers on the public status (home page, `/status`, `/api/v1/status`), such as `live,modes`; each other id takes `PORTAL_STATUS_<ID>_ADMIN_URL` and `PORTAL_STATUS_<ID>_ADMIN_SECRET_FILE`. Unset: `live` when the game admin API is set. See [server status](../docs/portal-status.md). |
| `PORTAL_STATUS_REFRESH_SECONDS` | no | Seconds between status polls of each game server, 5 to 300 (default 15). |
| `PORTAL_OPERATOR_CONTACT` | no | Contact text shown on the privacy page. |
| `PORTAL_REPORTS` | no | `on` (default) takes launcher reports and needs a report store; `off` answers `503 reports_disabled`. See [launcher reports](../docs/portal-reports.md). |
| `PORTAL_REPORT_STORE` | with reports on | Where report logs go: `s3` (production) or `dir` (tests and development). |
| `PORTAL_REPORT_S3_ENDPOINT`, `PORTAL_REPORT_S3_BUCKET` | for `s3` | The S3-compatible endpoint origin and the private bucket (path-style addressing). The bucket is checked at start. |
| `PORTAL_REPORT_S3_REGION` | no | Signing region (default `us-east-1`). |
| `PORTAL_REPORT_S3_ACCESS_KEY_FILE`, `PORTAL_REPORT_S3_SECRET_KEY_FILE` | for `s3` | Files holding the bucket's access key ID and secret key. Never logged. |
| `PORTAL_REPORT_DIR` | for `dir` | Existing, writable folder for report logs. |
| `PORTAL_REPORT_RETENTION_DAYS` | no | Days reports (rows and logs) are kept (default 30); purged hourly. |
| `PORTAL_LISTEN` | no | Listen address (default `0.0.0.0:8080`). Serve it behind a TLS-terminating proxy. |
| `PORTAL_DB_ACQUIRE_TIMEOUT_SECONDS` | no | Seconds a request waits for a database connection, 1 to 60 (default 3). After it, API requests answer `503 busy` and pages a "Busy" page, both with `Retry-After`. |
| `RUST_LOG` | no | Log filter (default `info`). |

Session and flow cookies use the `__Host-` prefix, so the site must be served over HTTPS at
the root of its own host name. `/healthz` returns 200 while the database answers.

## Launcher API

All bodies are JSON. Errors are `{"error": "<code>"}`; a body that is not the expected JSON gets `400 invalid_request`.
Any call may answer `503 busy` with `Retry-After: 2` while the database is unavailable (no free
connection within `PORTAL_DB_ACQUIRE_TIMEOUT_SECONDS`, a lost connection, or a failover), and
`500 server_error` when the portal fails otherwise; retry both later.

**Start a link** — `POST /api/v1/device/start`, body `{"client": "MAG launcher (Windows)"}`
(optional, at most 64 characters, shown to the player):

```json
{"device_code":"mgl_…","user_code":"BCDF-GHJK","verification_uri":"https://magtest.online/link",
 "verification_uri_complete":"https://magtest.online/link?code=BCDF-GHJK","expires_in":600,"interval":5}
```

Show `user_code` and `verification_uri` (or open `verification_uri_complete` in the browser).
`429 rate_limited` (more than 10 starts a minute from one address) and `503 busy` (too many links waiting) mean try again later.

**Wait for the player** — `POST /api/v1/device/token`, body `{"device_code": "mgl_…"}`, every
`interval` seconds:

| Response | Meaning |
| --- | --- |
| `200 {"device_token":"mgd_…","player_name":"Alpha"}` | Linked. Store the device token privately; it is shown once. |
| `400 authorization_pending` | Keep waiting. |
| `400 slow_down` | Polled too soon; wait longer. |
| `400 access_denied` | The player refused. |
| `400 expired_token` | The code expired; start again. |
| `400 invalid_grant` | Unknown or already used code. |

**Get a ticket** — `POST /api/v1/ticket` with `Authorization: Bearer mgd_…`, before each sign-in.
The body is optional: `{"audience": "test.magtest.online"}` asks for a ticket for that game server
(any case; it is lowercased). No body, an empty body, a body without `audience`, or a null or empty
`audience` means the default audience (`PORTAL_TICKET_AUDIENCE`), so launchers that send no body
keep working. The response's `audience` says which one the ticket is for:

```json
{"ticket":"<base64>","expires_at":1800000600,"account_id":42,"player_name":"Alpha","audience":"play.magtest.online"}
```

`401 invalid_token` means the launcher was unlinked: forget the token and link again.
`400 unknown_audience` means the portal does not issue tickets for that server, and
`400 invalid_request` that the body was not that JSON object.
`403 banned` (with `reason`) and `409 no_player_name` are shown to the player.

**Send a report** — `POST /api/v1/reports` with `Authorization: Bearer mgd_…`: a performance or
error report with gzip'd logs. [Launcher reports](../docs/portal-reports.md) has the body, limits and
responses (`201` stored, `200` already stored, `400 invalid_report`, `409 report_id_conflict`,
`413 too_large`, `429 rate_limited` with `Retry-After`, `503 reports_disabled`).

**Friends and presence** — `GET /api/v1/social`, `PUT /api/v1/social/presence` and the request,
friend, block and lookup calls under `/api/v1/social` with `Authorization: Bearer mgd_…`.
[Friends and presence](../docs/portal-social.md) has the contract and limits.

**Verifying keys** — `GET /api/v1/ticket-keys` returns `{"keys":[{"id":1,"alg":"ES256","spki":"<base64 DER>"}]}`.

## Admin panel

Staff use `https://<site>/admin`; everyone else gets 404 there. Roles follow the verified Google
email someone signs in with, so a role can be granted before the first sign-in.

| Role | Can |
| --- | --- |
| Owner (`PORTAL_OWNER_EMAILS`) | Everything below, and grant or remove admins. |
| Admin | Grant or remove moderators; reserve, release and assign names; delete accounts; set game character levels; download launcher report logs; everything a moderator can. |
| Moderator | Search and view accounts; ban and unban; unlink launchers; view game server characters; view launcher reports and their metrics. |

Staff act only on accounts and roles ranked below their own: nobody can change a peer, a
superior or themselves. Every change goes to an audit log shown on the admin overview.

The **Game server** section (`/admin/game`) lists the game server's characters with their level,
XP, skill points and session, through the game server's internal admin API. Setting a level is
two steps: a preview (a dry run on the game server) shows the change, and a separate confirmation
applies it and writes the audit log. The game server refuses while the player is signed in. Each
request is signed with HMAC-SHA256 over `METHOD\nPATH_AND_QUERY\nTIMESTAMP\nNONCE\nhex(SHA-256(body))`
and names the staff member in `X-Mag-Admin-Actor`; `src/game_admin.rs` documents the scheme and
holds a test vector.

## Operator commands

Run `mag-portal admin <command>` with `DATABASE_URL` set (in Kubernetes, `kubectl exec`):

| Command | Effect |
| --- | --- |
| `show <name>` | The account holding a name. |
| `find-email <email>` | Accounts signed in with an email address. |
| `ban <name> --reason <text>` / `unban <name>` | Stop or restore ticket issuing. Tickets already issued last until they expire. |
| `reserve <name>… [--reason <text>]` | Names nobody may claim, such as names that already have characters on the server. |
| `assign <name> --account-id <id>` | Give a reserved name to an account without a name. |
| `release <name>` | Let anyone claim a reserved name. |
| `reserved` | List reserved names. |
| `service-account create <name>` | Create a service account for a free or reserved name. |
| `service-account mint-token <name> --label <text>` | Mint a launcher token. Only the token goes to stdout, once; store it. |
| `service-account list` | Service accounts and their tokens (IDs, labels, use; never the tokens). |
| `service-account revoke-token <name> <device-id>` | Revoke one token. |
| `reports list`, `show`, `log`, `stats`, `delete` | Launcher reports; see [launcher reports](../docs/portal-reports.md#operator-commands). `log` and `delete` need the report store settings. |

The `service-account` commands, `reports log` and `reports delete` write the audit log as `operator (command line)`.

Banned accounts cannot delete themselves (that would shed the ban); delete them on request with SQL. Deleting an account (from its account page) keeps its name reserved, because the game server
keeps characters under the name. To move existing players over, reserve every name that has
characters, then assign each one when its owner signs in and asks (`find-email` finds them).

## Service accounts

Service accounts (admin panel: `/admin/service-accounts`) are fixed player identities for automation, such
as the test clients `PlayerLinux`, `PlayerRetail` and `Player`. They never sign in with Google: their
provider is `service` with a random subject, so no Google sign-in can match one, and the site never
honours a session for one. Staff mint their launcher tokens instead of linking a launcher. Admins
create one for a name that is free or reserved (taking over the reservation, keeping its stored
spelling), mint a labelled `mgd_` token (shown once on the result page, never stored or shown again,
only its hash is kept), revoke tokens and delete the account (its name stays reserved). Service
accounts get tickets through the same API as everyone else, and bans apply to them. Moderators can
see them, with their tokens' labels and use, but cannot change them, ban them or revoke their
tokens: they are operator infrastructure, so changing them needs an admin.
