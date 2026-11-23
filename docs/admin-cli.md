# Account and character maintenance (`mag-admin`)

The medius image includes `mag-admin`, an operator CLI for the local character
store. It runs inside the main medius container, reads the same `medius.json`
and catalog environment as the server, and changes the store only through the
server's own code: the same file lock, XP/level rules and atomic commit. It
starts no listeners and never rewrites `medius.json`.

```sh
M=deploy/<release>-mag-recomp-server-medius
kubectl exec -n <namespace> $M -c medius -- mag-admin accounts list
kubectl exec -n <namespace> $M -c medius -- mag-admin character show <account>
kubectl exec -n <namespace> $M -c medius -- mag-admin character set-level <account> 3           # dry run
kubectl exec -n <namespace> $M -c medius -- mag-admin character set-level <account> 3 --apply
```

## Commands

| Command | Effect |
| --- | --- |
| `accounts list` | Every account's character with level, XP, skill points and session state (`online`, `offline`, or `unknown` when no session marker exists). |
| `character show <account>` | One character in detail, including the XP needed for the next level. |
| `character set-level <account> <level> [--apply]` | Raises the character to `<level>` the way earned progression would. XP becomes that level's catalog threshold, and each gained level's `SkillPointsGained` is added. Without `--apply` it only prints the planned change. |

Exit codes: `0` success or dry run, `1` refused or failed, `2` usage or
configuration error. `--config DIR` selects a medius config directory other than
`/config/main`.

## Safety rules

- **Catalog-consistent.** The server refuses to load a store whose level doesn't
  match `LevelForExperience(experience)`, so levels are only set through the
  progression catalog. A store without a progression catalog is refused.
- **Raise only.** Lowering a level is refused, because skill points may already
  have been spent.
- **Logged-out accounts only.** Each medius process publishes its live login
  leases (pending and bound tickets, up to 120 s each) to
  `$MAG_ACTIVE_ACCOUNTS_DIR/mag-active-accounts.<role>.json` (default `/tmp`).
  `--apply` is refused while the account holds a live lease, and also when no
  marker exists, since session state can't be confirmed then. Dry runs still work
  and print a warning.
- **Backed up.** Before writing, the current store is copied to
  `.maintenance-backups/characters.before-admin-set-level-<account>-<level>-<sha12>.json`
  next to the store. To revert, copy that file back over the store while the
  account is logged out.
- **Serialized with the server.** Changes happen under the store's own lock, so a
  concurrent reward or loadout save is applied before or after, never lost.

Medius re-reads the store at each login, so the client shows the new level on
its next login; no restart is needed.

## Admin API

The same list, show and set-level operations are also available over an internal HTTP
API, so the accounts portal can offer them to staff. It runs inside the main medius
process (never MAPS) and calls the same code as `mag-admin`: the same rules,
session check, lock, backup and commit.

**Off by default.** It starts only when both variables are set:

| Variable | Value |
| --- | --- |
| `MAG_ADMIN_API_LISTEN` | `IP:port` to listen on, for example `0.0.0.0:10490` |
| `MAG_ADMIN_API_SECRET_FILE` | file holding the shared HMAC secret, at least 32 bytes after trimming surrounding whitespace; used as raw bytes, not decoded |

Setting only one, a short or unreadable secret, or setting them for a MAPS process
stops Medius at startup with an error that names the variables but never the secret.
With the Helm chart, set `adminApi` (see the chart README); it wires the variables, a
ClusterIP Service and a NetworkPolicy.

**Transport.** Plain HTTP/1.1, one request per connection, `Content-Length` bodies of
at most 16 KB (larger is `413`, chunked is `400`). Responses are `application/json`.

**Authentication.** Every request carries:

- `X-Mag-Admin-Timestamp`: Unix seconds, within 60 s of the server clock;
- `X-Mag-Admin-Nonce`: 16-64 characters of `[A-Za-z0-9_-]`, never reused within 5 minutes;
- `X-Mag-Admin-Actor`: 1-254 printable ASCII characters naming the staff member, logged but
  never used for authorization;
- `X-Mag-Admin-Signature`: lowercase hex HMAC-SHA256 with the secret over
  `METHOD\nPATH_AND_QUERY\nTIMESTAMP\nNONCE\nhex(SHA-256(body))`. The path is the request
  target exactly as sent, and an empty body hashes to
  `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.

Anything else is `401 {"error":"unauthorized"}`; the log records only the reason class.

**Endpoints.** `account` is the store key (the lowercase player name).

| Request | Success |
| --- | --- |
| `GET /admin/v1/accounts` | `200 {"accounts":[{"account","level","experience","skill_points","session"}]}` |
| `GET /admin/v1/characters/{account}` | `200 {"account","level","experience","skill_points","next_level_experience","session"}`; `next_level_experience` is null at the maximum level |
| `POST /admin/v1/characters/{account}/level` with `{"level":N,"apply":bool}` | `200 {"applied","account","from_level","to_level","experience_before","experience_after","skill_points_before","skill_points_after","backup"}`; `backup` is the backup file name, or null for a dry run |

`session` is `online`, `offline` or `unknown`, decided as `accounts list` decides it.
Set-level refusals are `409 {"error":code,"message":text}`:

| Code | When (`mag-admin` reason) |
| --- | --- |
| `online` | `apply` while the account holds a live login lease |
| `session_unknown` | `apply` with no session marker |
| `lower_level` | below the current level (`lowering-unsupported`) |
| `same_level` | already at that level (the CLI's `unchanged` no-op) |
| `level_out_of_range` | outside 1 to the catalog's maximum (`level-out-of-range`) |
| `no_progression_catalog` | the store has no progression catalog (`progression-catalog-unavailable`) |

A dry run while the account is online still answers `200` with `applied: false`. A bad
body is `400 {"error":"invalid_request"}`, an unknown account or path `404
{"error":"not_found"}`, a wrong method `405`, and a store or backup failure `500
{"error":"internal_error"}`. Every applied change is logged with the actor, account,
from and to levels, and backup name.

**Never route it through a Gateway.** The API can change any character, and its only
defence is the shared secret. Publishing it through a Gateway, Ingress or LoadBalancer
would expose it to the internet, put the signing scheme up against unlimited guessing,
and bypass the NetworkPolicy, which only sees in-cluster sources. Only the portal's
backend, inside the cluster, should reach it; staff reach the portal, never this port.
