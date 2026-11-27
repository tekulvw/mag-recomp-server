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
kubectl exec -n <namespace> $M -c medius -- mag-admin character kill <account> --reason "stuck in a parachute"           # dry run
kubectl exec -n <namespace> $M -c medius -- mag-admin character kill <account> --reason "stuck in a parachute" --apply
kubectl exec -n <namespace> $M -c medius -- mag-admin voice mute <account> --for 30m --reason "spam" --operator <you>          # dry run
kubectl exec -n <namespace> $M -c medius -- mag-admin voice mute <account> --for 30m --reason "spam" --operator <you> --apply
kubectl exec -n <namespace> $M -c medius -- mag-admin voice list
```

## Commands

| Command | Effect |
| --- | --- |
| `accounts list` | Every account's character with level, XP, skill points and session state (`online`, `offline`, or `unknown` when no session marker exists). |
| `character show <account>` | One character in detail, including the XP needed for the next level. |
| `character set-level <account> <level> [--apply]` | Raises the character to `<level>` the way earned progression would. XP becomes that level's catalog threshold, and each gained level's `SkillPointsGained` is added. Without `--apply` it only prints the planned change. |
| `character kill <account> [--reason TEXT] [--operator NAME] [--apply]` | Kills the account's player in its running match, exactly as if it had pressed Kill Self (see [Operator kill](#operator-kill)). Without `--apply` it only names the player, world and worker. It works through the running server's [admin API](#admin-api), so it needs `adminApi.enabled`. |
| `voice list` | Active voice mutes and bans: account, kind, end (or `until lifted`), when and by whom, and the reason. |
| `voice mute <account>`, `voice ban <account>` `[--for DURATION] [--reason TEXT] [--operator NAME] [--apply]` | Stops relaying the account's in-match voice (see [Voice moderation](#voice-moderation)). `--for` takes a whole number of minutes, hours, days or weeks (`30m`, `12h`, `7d`, `2w`, up to 3650 days); without it the sanction lasts until lifted. Without `--apply` it only prints the planned sanction. |
| `voice unmute <account>`, `voice unban <account>` `[--reason TEXT] [--operator NAME] [--apply]` | Lifts an active mute or ban. Without `--apply` it only prints the plan. |

Exit codes: `0` success or dry run, `1` refused or failed, `2` usage or
configuration error (for the voice commands also a bad `--for`, `--reason` or `--operator`). `--config DIR` selects a medius config directory other than
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

## Operator kill

`character kill` frees a live player who is stuck, for example in a parachute that
never lands or in a vehicle seat that no longer exists. Players have no key bound
to Kill Self, so the operator sends it for them. It is the only command that acts on
the running match rather than the character store, and it changes no stored data.

- **The same death as Kill Self.** Medius finds the account's placed player, asks
  the gameplay worker that owns that world, and the worker applies the player's own
  Kill Self (`0x0215`): the same death and death notice, the seat exit for a seated
  player, a downed player's give-up, then the ordinary respawn wave. Anything else a
  Kill Self death ends, such as an objective interaction in progress, ends the same
  way. The client sees an ordinary suicide. The match scoreboard
  counts a death and a suicide, as retail Kill Self does (a downed player's give-up
  counts no second death); no experience or stored statistic changes.
- **Through the running server.** The kill must reach the server that admitted the
  player, so the CLI signs a request to this container's own admin API with the same
  `MAG_ADMIN_API_LISTEN` and `MAG_ADMIN_API_SECRET_FILE` (connecting to loopback when
  it listens on every address). Without them it exits 2 and says so.
- **Audited.** `--operator` names the staff member (default `mag-admin:$USER`, or
  `mag-admin`); it becomes the API's actor. Medius logs every applied or refused kill
  with the actor, account, worker, world, player and reason, and the worker's event
  log records an `admin_kill` row with the operator and reason before the Kill Self
  event it applies.
- **Workers that can do it.** Medius only asks a worker whose hello lists
  `admin_kill_v1` ([retail-runtime.md](retail-runtime.md#operator-kill-admin_kill_v1)).
  An older worker is refused at Medius with `worker_unsupported` and never sees the
  request.
- **Never retried.** A request still waiting behind the player's queued events after
  2 s is withdrawn unsent. If the worker does not answer, the result is
  `worker_unanswered`: the kill may or may not have applied, so look at the player
  before retrying.

Refusals print `refused (<code>)` and exit 1:

| Code | When |
| --- | --- |
| `not_found` | No character has that account name |
| `not_online` | The account holds no live login lease |
| `not_in_match` | Online, but not a placed, connected player of a running match: in the lobby or queue, still joining, on the results screen, or leaving |
| `not_alive` | Already dead or waiting for its first spawn; it needs no kill |
| `match_finished` | The match has ended; the player moves to results anyway |
| `worker_unsupported` | The worker that owns the player does not advertise `admin_kill_v1` |
| `worker_unavailable` | The world stream is down, or the request could not be queued or sent; nothing changed |
| `worker_unanswered` | Sent, but no valid answer; the outcome is unknown |
| `kill_refused` | The worker applied the request but denied the Kill Self itself (for example `mounted_death_not_supported`); `worker_reason` names the denial |
| `worker_refused` | The worker refused the request with another code, named in `worker_reason` |

## Voice moderation

The voice relay ([voice relay design](superpowers/specs/2026-11-25-voice-relay-design.md#moderation))
drops every voice packet an account with an active **voice mute** or **voice ban** sends: both the
copy for its talk channel and the proximity copy, counted as `muted` in the `Voice relay:` line. The
player still hears everyone and is not told. Retail and the client have no listener-side difference
between the two, so they act alike; the kind records intent (a mute is a cool-off, a ban a standing
refusal). Either ends at its `--for` time or lasts until lifted.

To mute someone:

1. Find the account (`accounts list`, or the portal's game admin page).
2. Dry run: `mag-admin voice mute <account> --for 30m --reason "spam" --operator <you>`. It prints
   the sanction before and after.
3. Apply: add `--apply`. The running server picks it up within 5 seconds (at once when changed
   through the admin API or the portal); a player in a match goes silent mid-sentence.
4. Check with `voice list`; lift it early with `voice unmute <account> --apply`.

Rules:

- **Per account and kind.** An account has at most one mute and one ban. Imposing a kind again
  replaces it (new end, reason and operator). Lifting needs an active sanction of that kind
  (`not_sanctioned` otherwise); an expired one needs no lifting.
- **Any session state.** Unlike `set-level`, sanctions apply while the account is online. The account
  must have a character (`not_found` otherwise).
- **Journalled.** `--operator` names the staff member (default `mag-admin:$USER`, or `mag-admin`).
  With the PostgreSQL store each applied change is a `mag.admin_changes` row (operation `voice-mute`,
  `voice-ban`, `voice-unmute` or `voice-unban`) with the sanction before and after, the operator and
  the reason, in the same transaction. With the JSON store the file before the change is kept in
  `.maintenance-backups/characters.before-admin-voice-<operation>-<account>-<sha12>.json`.
- **Cached, never per packet.** Main Medius loads active sanctions only when it runs the relay
  (`MAG_LOCAL_DME=1` and `MAG_VOICE_RELAY=1`), reloads them every 5 s and after each API change, and
  checks expiry at each lookup. A failed reload keeps the previous set and logs
  `Voice sanctions: reload failed` once.

Refusals print `refused (<code>)` and exit 1: `not_found` (no character has that account) and
`not_sanctioned` (nothing to lift).

## PostgreSQL character store

With `MAG_CHARACTER_STORE=postgres` (chart `characterStore.backend: postgres`), the
commands above work unchanged against PostgreSQL. `character-store` commands
manage that database. They print counts, a digest of the canonical contents, and
record and field names; never player data or credentials.

| Command | Effect |
| --- | --- |
| `character-store status` | The backend and, for PostgreSQL, its host, database and schema version, each table's row count, the contents' counts and digest, and when the last import wrote. |
| `character-store migrate` | Apply pending schema migrations (main Medius also does this at startup). |
| `character-store import --from <json> [--dry-run] [--replace [--discard-postgres-changes]]` | Copy a JSON store into PostgreSQL in one transaction, then verify. The JSON file stays locked meanwhile, and a JSON store with fields this build doesn't know is refused. See below. |
| `character-store verify --json <json> [--as-export]` | Compare a JSON store with PostgreSQL: both stores' counts and digests, then characters (unset XP and level read as 0 and 1), receipts and match rows (without their dates). Differences are listed by record and field name; exit 1 if any. With `--as-export`, check a file `export` wrote: settled history beyond the JSON bound may be missing (it is counted as `trimmed`), everything else must match. See below. |
| `character-store export --to <json> [--replace]` | Write PostgreSQL to a JSON store, the rollback path, then read the file back and verify it against what was read; exit 1 if it is not a complete copy. Settled history beyond the JSON bound is left out. An existing file is replaced only with `--replace`, which first keeps it as `<json>.before-export-<UTC time>`. |
| `character-store prune` | Run reward-ledger retention now (main Medius runs it hourly). |

`import` is idempotent. It prints both stores' counts and digests, the last import and the
action it takes:

- **Empty database:** it imports.
- **Same contents as the JSON store:** it writes nothing and reports `unchanged`, so a repeated
  import is safe.
- **Different contents:** it refuses, unless `--replace` is given. `--replace` deletes the database's
  characters (with their kits, match rows, receipts and history) and imports again in the same
  transaction, but only if nothing was written after the last import. The server's own writes, a
  retention prune or an operator change all count. To discard those too, add
  `--discard-postgres-changes`, after an `export` if they matter.

`--dry-run` reads the JSON file without locking it, prints the same report and writes nothing. It
exits 1 when the import would be refused. Every import that writes records its counts and digest,
never player data, in `mag.admin_changes` (operation `character-store-import`).
While it runs, the import holds an exclusive lock on `mag.characters`, so no character write can
interleave.

PostgreSQL keeps a week of match rows and receipts (`MAG_REWARD_LEDGER_RETENTION_DAYS`), more
than the JSON store's bound (1 MiB, and 4096 receipts or match rows). `export` therefore writes
everything the JSON store must hold: every character with its kits and progression, voice
sanctions, and every open match row with its receipts. It then adds settled history (finalized
match rows, and open ones with no reward for the open-row TTL), most recently rewarded first, until
the next row would pass the bound. It does not compact to the live store's smaller target. If the
rows the JSON store must hold do not fit, the export fails. Otherwise it reads the file back with
the JSON store's own validation, as a rollback would, and compares it with the database contents it
was written from. Those are read in one transaction, so writes by the running server don't count.
It prints `trimmed: N settled match rows with M reward receipts` for what it left out, then
`verified`, or the differences and exit 1.

`verify --json <file> --as-export` makes the same comparison against the database as it is now.
Trimmed history is excused, but nothing else is, so rewards committed since the export show up as
differences. Plain `verify` stays exact, as an import needs.

Cutover, with no matches running:

1. Stage the database while the server stays on JSON: set
   `characterStore.postgres.existingSecret` (and `caSecret`) but keep
   `characterStore.backend: json`. The chart then mounts the database settings
   without switching the store.
2. Keep a copy, migrate and import:

   ```sh
   M=deploy/<release>-mag-recomp-server-medius
   PG="env MAG_CHARACTER_STORE=postgres mag-admin character-store"
   kubectl exec -n <namespace> $M -c medius -- cp /accounts/characters.json /accounts/characters.before-postgres.json
   kubectl exec -n <namespace> $M -c medius -- $PG migrate
   kubectl exec -n <namespace> $M -c medius -- $PG import --from /accounts/characters.json --dry-run
   kubectl exec -n <namespace> $M -c medius -- $PG import --from /accounts/characters.json
   ```

3. Just before switching, run `verify --json /accounts/characters.json` the same
   way. It must report no differences. Anything committed to the JSON store after
   the import shows up here; `import --replace` then brings the database up to date.
4. Set `characterStore.backend: postgres` and roll Medius.

To roll back, stop play, run
`mag-admin character-store export --to /accounts/characters.json --replace`, then
set `characterStore.backend: json`. To cut over again later, import with
`--replace --discard-postgres-changes`. The database then holds rows the server wrote
before the rollback, which the export already carried into the JSON store.

## Admin API

The same list, show, set-level, kill and voice operations are also available over an internal HTTP
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
| `POST /admin/v1/characters/{account}/kill` with `{"apply":bool,"reason":text}` | `200 {"applied","account","worker_id","world_id","player_id","result","reason"}`: the [operator kill](#operator-kill). `result` is `killed` or `gave_up` (a downed player), null for a dry run. `reason` is optional, printable ASCII, at most 256 characters; the actor header is the operator |
| `GET /admin/v1/voice` | `200 {"sanctions":[{"account","kind","reason","operator","created_at","expires_at"}]}`: the active [voice sanctions](#voice-moderation), by account, a ban before a mute. Times are UTC `YYYY-MM-DDTHH:MM:SSZ`; `expires_at` and `reason` are null when absent |
| `POST /admin/v1/voice/{account}/mute` or `/ban` with `{"apply":bool,"reason":text,"duration_minutes":N}` | `200 {"applied","account","action","kind","before","after"}`; `before` is the active sanction of that kind it replaces, `after` the new one, each null or a sanction as above. `reason` and `duration_minutes` (1 to 5256000) are optional; without a duration it lasts until lifted. The actor header is the operator |
| `POST /admin/v1/voice/{account}/unmute` or `/unban` with `{"apply":bool,"reason":text}` | `200`, as above with `after` null. `409 {"error":"not_sanctioned","message"}` when there is no active sanction of that kind |
| `GET /admin/v1/dme-ports` | `200 {"capacity","in_use","available","first_port","last_port","source","udp_ports_bound"}`: humans holding a DME UDP port across all worlds against the [cap](configured-world-registry.md#dme-port-capacity). `capacity`, `available` and `last_port` are null when unbounded; `source` is `port_range`, `override` or `unbounded`; `udp_ports_bound` counts sockets actually bound |
| `GET /admin/v1/status` | `200`: the read-only server status below, for the portal's public status page |

`session` is `online`, `offline` or `unknown`, decided as `accounts list` decides it.

`GET /admin/v1/status` carries counts only, never an account, character or player name, so
the portal can publish it:

```json
{"medius_version":"0.11.0","gameplay_version":"0.15.0","started_at":"2026-11-26T10:00:00Z",
 "uptime_seconds":7200,"mode":"registry","players_online":12,"queued":1,"queued_by_game_type":[1,0,0],
 "dme_ports":{"capacity":32,"in_use":9},"human_limit":32,
 "worlds":[{"worker_id":"sabotage","state":"open","game_types":[0],"human_capacity":64,"human_limit":32,
   "world_size":64,"bot_fill":64,"pending":0,
   "match":{"profile_id":"…","game_type":0,"attacking_faction":1,"defending_faction":2,"humans":8,
            "attacking_humans":5,"defending_humans":3,"human_capacity":64,"human_limit":32,"bots":56,
            "bot_fill":64,"world_capacity":64}}]}
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
- Human limits. The top-level `human_limit` is the server-wide cap on humans at once across every
  world: the DME port capacity (`dme_ports.capacity`), null when unbounded. A world's (and its
  match's) `human_limit` is the humans it can actually hold: its configured row and profile cap,
  within that server-wide cap. `human_capacity` keeps the row and profile cap alone for older
  readers: a 64-Character Sabotage row reports 64 there although 32 DME ports hold only 32
  humans. `world_size` is the world's Characters, humans and bots (the running match's, else the
  largest profile's); `bot_fill` is the population bots fill it to, humans included, or null
  without bot fill.
Set-level refusals are `409 {"error":code,"message":text}`:

| Code | When (`mag-admin` reason) |
| --- | --- |
| `online` | `apply` while the account holds a live login lease |
| `session_unknown` | `apply` with no session marker |
| `lower_level` | below the current level (`lowering-unsupported`) |
| `same_level` | already at that level (the CLI's `unchanged` no-op) |
| `level_out_of_range` | outside 1 to the catalog's maximum (`level-out-of-range`) |
| `no_progression_catalog` | the store has no progression catalog (`progression-catalog-unavailable`) |

Kill refusals use the codes in [Operator kill](#operator-kill), as
`{"error":code,"message":text,"worker_id","world_id","player_id","worker_reason"}` (the
location fields are null until the player is found): `404 {"error":"not_found"}` for an unknown
account, `503` for `worker_unavailable` and `worker_unanswered`, and `409` for the others. A
kill body with fields other than `apply` and `reason` is `400`.

Voice requests answer `404 {"error":"not_found"}` for an account without a character and `400
{"error":"invalid_request"}` for a body with other fields, a bad `reason` or duration, or a
duration on a lift. Each applied or refused voice change is logged with the actor, account, end
and reason, and an applied one asks the relay to reload its sanctions in the background; the answer
never waits for that reload.

A voice change commits on the PostgreSQL primary without waiting for a synchronous standby
(`synchronous_commit = local` for that transaction), so it answers in milliseconds even while the
standby is slow; the standby still receives it by streaming. Character, level and reward commits
keep waiting for the standby.

If the database stops answering during the commit itself (after every statement ran), the change
may be saved: any write answers `503 {"error":"outcome_unknown","message":text}` and the log says
`outcome unknown`. Read the state back (`voice list`, the character) before retrying.

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
