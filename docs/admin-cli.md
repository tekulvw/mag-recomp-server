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
  With the PostgreSQL store, the character row before and after the change is
  recorded in `mag.admin_changes` in the same transaction instead.
- **Serialized with the server.** Changes happen under the store's own lock, so a
  concurrent reward or loadout save is applied before or after, never lost.

Medius re-reads the store at each login, so the client shows the new level on
its next login; no restart is needed.

## PostgreSQL character store

With `MAG_CHARACTER_STORE=postgres` (chart `characterStore.backend: postgres`), the
commands above work unchanged against PostgreSQL. `character-store` commands
manage that database. They print counts, a digest of the canonical contents, and
record and field names; never player data or credentials.

| Command | Effect |
| --- | --- |
| `character-store status` | The backend and, for PostgreSQL, its host, database and schema version. |
| `character-store migrate` | Apply pending schema migrations (main Medius also does this at startup). |
| `character-store import --from <json>` | Copy a JSON store into the **empty** PostgreSQL store in one transaction, then verify. The JSON file stays locked meanwhile. A non-empty database, or a JSON store with fields this build doesn't know, is refused. |
| `character-store verify --json <json>` | Compare a JSON store with PostgreSQL: characters (unset XP and level read as 0 and 1), receipts and match rows (without their dates). Differences are listed by record and field name; exit 1 if any. |
| `character-store export --to <json> [--replace]` | Write PostgreSQL to a JSON store, the rollback path. Settled history is compacted to fit the JSON bound. An existing file is replaced only with `--replace`, which first keeps it as `<json>.before-export-<UTC time>`. |
| `character-store prune` | Run reward-ledger retention now (main Medius runs it hourly). |

Cutover, with no matches running:

1. Stage the database while the server stays on JSON: set
   `characterStore.postgres.existingSecret` (and `caSecret`) but keep
   `characterStore.backend: json`. The chart then mounts the database settings
   without switching the store.
2. Keep a copy, migrate and import:

   ```sh
   M=deploy/<release>-mag-recomp-server-medius
   kubectl exec -n <namespace> $M -c medius -- cp /accounts/characters.json /accounts/characters.before-postgres.json
   kubectl exec -n <namespace> $M -c medius -- env MAG_CHARACTER_STORE=postgres mag-admin character-store migrate
   kubectl exec -n <namespace> $M -c medius -- env MAG_CHARACTER_STORE=postgres mag-admin character-store import --from /accounts/characters.json
   ```

3. Just before switching, run `verify --json /accounts/characters.json` the same
   way. It must report no differences; anything committed to the JSON store after
   the import shows up here.
4. Set `characterStore.backend: postgres` and roll Medius.

To roll back, stop play, run
`mag-admin character-store export --to /accounts/characters.json --replace`, then
set `characterStore.backend: json`.

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
| `GET /admin/v1/dme-ports` | `200 {"capacity","in_use","available","first_port","last_port","source","udp_ports_bound"}`: humans holding a DME UDP port across all worlds against the [cap](configured-world-registry.md#dme-port-capacity). `capacity`, `available` and `last_port` are null when unbounded; `source` is `port_range`, `override` or `unbounded`; `udp_ports_bound` counts sockets actually bound |
| `GET /admin/v1/status` | `200`: the read-only server status below, for the portal's public status page |

`session` is `online`, `offline` or `unknown`, decided as `accounts list` decides it.

`GET /admin/v1/status` carries counts only, never an account, character or player name, so
the portal can publish it:

```json
{"medius_version":"0.11.0","gameplay_version":"0.15.0","started_at":"2026-11-26T10:00:00Z",
 "uptime_seconds":7200,"mode":"registry","players_online":12,"queued":1,"queued_by_game_type":[1,0,0],
 "dme_ports":{"capacity":32,"in_use":9},
 "worlds":[{"worker_id":"sabotage","state":"open","game_types":[0],"human_capacity":32,"pending":0,
   "match":{"profile_id":"…","game_type":0,"attacking_faction":1,"defending_faction":2,"humans":8,
            "attacking_humans":5,"defending_humans":3,"human_capacity":32,"bots":56,"world_capacity":64}}]}
```

- `medius_version` and `gameplay_version` are the newest releases in the Medius changelog built
  into the image and the gameplay changelog the chart copies from the gameplay image (the same
  notes the in-game NEWS shows); null without them. `started_at` is when this process started.
- `mode` is `registry` (configured worlds), `single` (one local world, reported as `gameplay`) or
  `none` (no local gameplay).
- `players_online` counts accounts holding a live login lease, as `accounts list` decides
  `online`; null when no session marker can be read.
- `queued_by_game_type` counts humans in the registry's global queue by requested game type
  (Sabotage, Acquisition, Domination); `pending` counts humans already placed on a world and not
  yet admitted there.
- A world's `state` is `open` (admitting), `draining` (closed to new players by the registry's
  `draining`, matches continue), `faulted` (closed after a worker fault: unreachable at start, a
  failed tick, a foreign or refused incarnation, or the adoption cap; Medius logs these as
  "Configured worker …", and the entry reopens only when a matching worker is adopted) or
  `unavailable` (not faulted, but not ready: starting, or its world stream lapsed).
- `match` is null unless humans are in a match. Factions are 0 Valor, 1 Raven, 2 SVER; game
  types 0 Sabotage, 1 Acquisition, 2 Domination. `bots` is the profile's bot fill less the
  humans, the population the worker tops the world up to; 0 without bot fill.
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
