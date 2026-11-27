# PostgreSQL character store

Date: 2026-11-25. Status: design for review. The user approved writing it. Implementation lands
behind `MAG_CHARACTER_STORE=json|postgres` (default `json`) and reaches the test server only after
the user has reviewed this design. The database is PostgreSQL on the cluster's CloudNativePG (CNPG)
operator (user decision, 2026-11-25).

## Why

Medius keeps every character, reward receipt and match-progress row in one JSON file,
`/accounts/characters.json`, through `LocalCharacterStore`
(`horizon/Server.Medius/Medius/LocalCharacterStore.cs`). The store has no in-memory copy. Every
operation, including each reward, takes an exclusive lock file (`characters.json.lock`), reads
and validates the whole file, and rewrites it through a temporary file, `fsync`, rename and
directory `fsync`. A hard bound of 1 MiB protects that cycle.

On 2026-11-25 the live store reached the bound. 1,756 receipts had never been pruned, so every
reward commit failed with `Character store exceeds local bound`. Medius kept deferring the
rewards, gameplay never received `progression_committed`, and every match hung at the winner
banner without XP. A manual prune mitigated it. Two merged changes then bounded the ledger:

- #301 compacts finalized match rows when the file passes 3/4 MiB.
- #305 dates rows (`lastRewardAt`) and also compacts open rows that have had no reward for
  `MAG_REWARD_LEDGER_OPEN_MATCH_TTL_HOURS` (default 24).

`horizon/README.md`, "Reward ledger bounds", documents both.

Those changes keep the file under its bound. They don't change its shape: every reward still
rewrites the whole file, and all worlds serialize on one lock. At 381 KB today, each reward
writes about 0.4 MB to the Synology-backed volume. Larger lobbies multiply that. A 128- or
256-player lobby produces hundreds of terminal rewards in the seconds after a match result, plus
a steady flow of action rewards from three or more worlds. Each one would rewrite up to 3/4 MiB.

Live shape on 2026-11-25 (read-only, counts only): schema 3, 9 characters, 621 receipts and 16
match rows (4 finalized). Receipt keys are 52 to 72 characters and match keys 32 to 72. The
average serialized receipt is about 380 bytes. Kills make up most receipts (594 of 621).

## Goals

1. **No size cap and no whole-file rewrites.** A reward writes one receipt, one match row and
   one character row.
2. **Safe concurrent commits** from three or more worlds now, and from 128- or 256-player
   lobbies later. Commits for different characters don't wait on each other in the database.
3. **The portal can read levels, XP and match history.**
4. **Exactly-once XP under gameplay's resend loop.** This holds after Medius restarts, database
   failovers, lost acknowledgements and uncertain commits.
5. **Multi-Medius stays possible.** Nothing in the schema or write path assumes a single
   writer process.

## Non-goals

- Changing any progression rule: XP, levels, skill points, FRAGO, leadership, Training or
  loadout ownership. The rule code stays as it is and the backends share it (see
  [Code shape](#code-shape)).
- Changing gameplay's reward protocol, its in-memory outbox or its resend cadence.
- Multi-Medius itself. World ownership, reservations and session leases stay in one process.
- Moving portal data (accounts, links, names) into this database, or game data into the
  portal's database.
- Dropping the JSON backend. It stays the default until the PostgreSQL backend has run on the
  test server and live, and it remains the rollback for one release afterwards.

## How rewards flow today

1. Gameplay creates a reward with a stable `reward_id` for each event: a kill, heal, repair,
   objective or match result. It keeps the reward in an in-memory outbox (`pending_rewards`,
   at most 256 ordinary rewards plus terminal rewards per admitted human). It sends the outbox
   in batches of up to 32 and resends every unacknowledged reward each second.
2. Medius (`LocalDmeHost.SettleRewardsAsync`) checks each reward against the authenticated
   admission. Under the registry-wide `_progressionGate` it calls
   `ApplyRewardForAccount(account, characterId, rewardId, "world-W-<match>", kind, units, ...)`.
3. That call returns the stored snapshot unchanged if a receipt for `rewardId` exists (a
   duplicate). Otherwise it computes the award from the match row and the catalog. It then
   writes the character, the match row and a new receipt in one atomic file commit.
4. Only after the commit returns does Medius send `progression_committed` to gameplay, which
   drops the reward from its outbox.
5. If the commit throws `IOException` or `InvalidDataException`, Medius logs
   `Gameplay reward persistence deferred` and stops the batch. Gameplay resends next second.

The receipt is the idempotency key. A crash between commit and acknowledgement causes a resend,
which then replays as a duplicate. **Gameplay's outbox already buffers every unpersisted
reward,** and the PostgreSQL design keeps that contract unchanged.

## Schema

All tables live in one schema, `mag`, owned by the application role. Identifiers in the
protocol are `uint32`; they are stored as `bigint` with range checks. Timestamps are
`timestamptz`. The JSON store's nullability is kept where the code gives `null` a meaning (for
example, a `null` skill-point balance means "the catalog's initial points").

```sql
CREATE TABLE mag.characters (
  id                        bigint PRIMARY KEY CHECK (id BETWEEN 1 AND 4294967295),
  account                   text NOT NULL UNIQUE,        -- portal account name; one character per account until migration 0003 (docs/character-slots.md)
  name                      text NOT NULL CHECK (name ~ '^[!-~]{1,32}$'),
  faction                   smallint NOT NULL CHECK (faction BETWEEN 0 AND 2),
  head_id                   smallint NOT NULL CHECK (head_id BETWEEN 0 AND 255),
  voice_id                  smallint NOT NULL CHECK (voice_id BETWEEN 0 AND 255),
  flags                     smallint NOT NULL DEFAULT 0 CHECK (flags BETWEEN 0 AND 255),
  universe_id               bigint NOT NULL,
  np_id                     bytea NOT NULL CHECK (octet_length(np_id) = 36),
  experience                integer CHECK (experience >= 0),
  level                     smallint CHECK (level >= 1),
  skill_points              smallint CHECK (skill_points BETWEEN 0 AND 255),
  owned_skills              bigint[] CHECK (cardinality(owned_skills) = 5),
  unlocked_items            bigint[] CHECK (cardinality(unlocked_items) = 24),
  -- Loadout preferences
  last_used_loadout         smallint NOT NULL DEFAULT 0 CHECK (last_used_loadout BETWEEN 0 AND 4),
  loadout_layout_revision   numeric(20,0) NOT NULL DEFAULT 0,  -- ulong
  -- Leadership and nominations (all null = no leadership record yet)
  leadership_level          smallint,
  nominations               smallint,
  matches_since_last_command smallint,
  leadership_points         bigint CHECK (leadership_points BETWEEN 0 AND 4294967295),
  leadership_level_floor    smallint,
  -- Bookkeeping
  revision                  bigint NOT NULL DEFAULT 1,  -- +1 on every write; for caches and audits
  created_at                timestamptz NOT NULL DEFAULT now(),
  updated_at                timestamptz NOT NULL DEFAULT now(),
  CHECK ((leadership_level IS NULL) = (nominations IS NULL)
     AND (nominations IS NULL) = (matches_since_last_command IS NULL)),
  CHECK (leadership_level_floor IS NULL OR
         (leadership_points IS NOT NULL AND leadership_level_floor = leadership_level))
);
CREATE UNIQUE INDEX characters_name_key ON mag.characters (lower(name));  -- names are ASCII
CREATE SEQUENCE mag.character_ids OWNED BY mag.characters.id;             -- set past the import's max

CREATE TABLE mag.character_loadouts (               -- exactly 0 or 5 rows per character
  character_id bigint   NOT NULL REFERENCES mag.characters ON DELETE CASCADE,
  slot         smallint NOT NULL CHECK (slot BETWEEN 0 AND 4),
  update_id    smallint NOT NULL CHECK (update_id BETWEEN 0 AND 255),
  items        smallint[] NOT NULL CHECK (cardinality(items) = 22),
  PRIMARY KEY (character_id, slot)
);

CREATE TABLE mag.match_progress (                   -- one character's progress in one match
  character_id   bigint  NOT NULL REFERENCES mag.characters ON DELETE CASCADE,
  match_key      text    NOT NULL CHECK (match_key ~ '^[!-~]{1,256}$'),
  experience     integer NOT NULL CHECK (experience >= 0),
  fraction       real    NOT NULL CHECK (fraction >= 0 AND fraction < 1),
  frago_points   integer CHECK (frago_points >= 0 AND frago_points <= experience),
  finalized      boolean NOT NULL DEFAULT false,
  result_kind    text,                              -- the final reward's kind, such as match_major_victory
  first_reward_at timestamptz NOT NULL,
  last_reward_at  timestamptz NOT NULL,
  PRIMARY KEY (character_id, match_key)
);
CREATE INDEX match_progress_last_reward ON mag.match_progress (last_reward_at);

CREATE TABLE mag.reward_receipts (
  reward_id    text   PRIMARY KEY CHECK (reward_id ~ '^[!-~]{1,256}$'),  -- unique: replays are idempotent
  character_id bigint NOT NULL,
  match_key    text   NOT NULL,
  kind         text   NOT NULL,
  units        integer NOT NULL,
  leadership   jsonb,                -- GameplayLeadershipResult, when the reward carried one
  frago        boolean,
  outcome      jsonb  NOT NULL,      -- the exact ProgressionRewardOutcome returned on replay
  experience_awarded integer GENERATED ALWAYS AS ((outcome->>'experienceAwarded')::integer) STORED,
  committed_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (character_id, match_key) REFERENCES mag.match_progress ON DELETE CASCADE
);
CREATE INDEX reward_receipts_row ON mag.reward_receipts (character_id, match_key);

CREATE TABLE mag.match_history (                    -- summaries of pruned match rows, kept long term
  character_id   bigint NOT NULL REFERENCES mag.characters ON DELETE CASCADE,
  match_key      text   NOT NULL,
  experience     integer NOT NULL,
  frago_points   integer,
  finalized      boolean NOT NULL,                  -- false: left early, or the match was abandoned
  result_kind    text,
  rewards        integer NOT NULL,                  -- receipt count at prune time
  first_reward_at timestamptz NOT NULL,
  last_reward_at  timestamptz NOT NULL,
  PRIMARY KEY (character_id, match_key)
);

CREATE VIEW mag.character_matches AS                -- what the portal reads, through the admin API
  SELECT p.character_id, p.match_key, p.experience, p.frago_points, p.finalized, p.result_kind,
         (SELECT count(*) FROM mag.reward_receipts r
           WHERE r.character_id = p.character_id AND r.match_key = p.match_key) AS rewards,
         p.first_reward_at, p.last_reward_at
    FROM mag.match_progress p
  UNION ALL
  SELECT character_id, match_key, experience, frago_points, finalized, result_kind, rewards,
         first_reward_at, last_reward_at
    FROM mag.match_history;

CREATE TABLE mag.admin_changes (                    -- replaces set-level's file backups
  id         bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  at         timestamptz NOT NULL DEFAULT now(),
  operation  text   NOT NULL,                       -- such as set-level
  account    text   NOT NULL,
  before     jsonb  NOT NULL,                       -- the character row before the change
  after      jsonb  NOT NULL
);
```

Notes:

- **Receipt keys stay global.** The JSON store keys receipts by `rewardId` alone and checks the
  identity fields on replay (`reward-receipt-mismatch`). The primary key on `reward_id` keeps
  exactly that.
- **The outcome is stored as JSON** because replay must return the original response exactly,
  including fields that later builds add. `experience_awarded` is generated from it for history
  queries, so there is one source of truth.
- **Row recency is a timestamp.** The JSON store uses dictionary order to record which match row
  was touched last. Here `last_reward_at` replaces it.
- **No extension-data columns.** The JSON classes keep unknown fields through
  `[JsonExtensionData]` so that older builds don't drop newer fields. In PostgreSQL, a new field
  is a migration. The import refuses a JSON store that carries unknown fields; the live store
  has none.
- **The 128-character cap is dropped** for this backend. It existed to keep the file under
  1 MiB. Accounts are gated by the portal.
- **Validation becomes per row.** Today, one character whose level doesn't match its XP makes
  `ReadDocument` refuse the whole store, so every player is affected. With this backend the
  same check runs when a character is read, and a bad row affects only its own account. The
  ledger cross-checks in `ValidateRewardLedger` become foreign keys and CHECK constraints, plus
  the verify command below.

### Migrations

The schema is versioned as numbered SQL files embedded in Server.Medius
(`CharacterStore/Migrations/0001_initial.sql`, and so on), as the portal does with sqlx. A small
in-repository runner applies them:

- A `mag.schema_migrations (version, name, sha256, applied_at)` table records each applied file.
- The runner takes `pg_advisory_lock` on a fixed key, so concurrent Medius or MAPS starts apply
  each migration once.
- Each migration runs in its own transaction.
- At startup the runner refuses to continue if an applied migration's checksum has changed,
  or if the database has a version this build doesn't know (a downgrade). In both cases it
  names the version. A downgrade is done by switching the flag back to `json` (see
  [Rollback](#rollback)), never by running an older build against a newer schema.
- Main Medius applies migrations at startup. MAPS and `mag-admin` only check the version and
  refuse a schema that doesn't match.
- `mag-admin character-store migrate [--dry-run]` applies them by hand.

EF Core migrations, DbUp or FluentMigrator would add a dependency and an ORM model for six
tables. The runner needs about 100 lines and plain SQL that reviewers can read. The only new
package is **Npgsql** (PostgreSQL License), which is added to `THIRD-PARTY-NOTICES.md`.

## Code shape

Today one class holds both the rules and the persistence. The implementation splits it:

- **`CharacterRules`** holds the existing pure logic: creation, loadout save, delete and
  selection, skill purchase, nominations, rewards (`TryAward`, FRAGO, leadership), Training and
  admin level changes. Each operation takes the loaded character (and, for rewards, its match
  row and any existing receipt) and returns either a refusal or the new state plus its outcome.
  The code moves; it isn't rewritten.
- **`ICharacterStore`** is the public surface Medius uses today (`ReadForAccount`,
  `CreateForAccount`, `ApplyRewardForAccount`, and so on). It has two implementations:
  - `JsonCharacterStore`: today's `LocalCharacterStore`, unchanged in behaviour. It keeps the
    lock file, whole-document commit and #301/#305 compaction.
  - `PostgresCharacterStore`: loads the rows an operation needs, applies `CharacterRules`, and
    writes the result in one transaction.
- `Program.CreateCharacterStore()` picks one from `MAG_CHARACTER_STORE`. Startup fails, naming
  the variable, on any other value, or on `postgres` without connection settings.

Both backends therefore make the same decisions. One synthetic suite runs against both (see
[Test plan](#test-plan)).

## Write path

### One reward, one transaction

`ApplyRewardForAccount` on PostgreSQL runs at `READ COMMITTED`:

```sql
BEGIN;
-- 1. Serialize this character. Commits for other characters proceed in parallel.
SELECT ... FROM mag.characters WHERE id = $character AND account = $account FOR UPDATE;
SELECT ... FROM mag.character_loadouts WHERE character_id = $character;   -- for record projection
-- 2. Replay check: an existing receipt returns its stored outcome, marked duplicate.
SELECT ... FROM mag.reward_receipts WHERE reward_id = $reward;
-- 3. The match row (covered by the character lock).
SELECT ... FROM mag.match_progress WHERE character_id = $character AND match_key = $match;
-- (Medius computes the award with CharacterRules, exactly as today.)
-- 4. Write.
INSERT INTO mag.match_progress (...) VALUES (...)
  ON CONFLICT (character_id, match_key) DO UPDATE
  SET experience = EXCLUDED.experience, fraction = EXCLUDED.fraction,
      frago_points = EXCLUDED.frago_points, finalized = EXCLUDED.finalized,
      result_kind = EXCLUDED.result_kind, last_reward_at = EXCLUDED.last_reward_at;
INSERT INTO mag.reward_receipts (...) VALUES (...)
  ON CONFLICT (reward_id) DO NOTHING RETURNING reward_id;
-- No row returned: another transaction committed this reward_id first (it can only race
-- if it belongs to a different character, because step 1 serializes one character).
-- ROLLBACK and rerun from step 1, which then takes the replay path.
UPDATE mag.characters SET experience = ..., level = ..., skill_points = ..., leadership_... ,
       revision = revision + 1, updated_at = now() WHERE id = $character;
COMMIT;
```

Npgsql sends steps 1 to 3 as one batch and step 4 plus `COMMIT` as a second, so a reward costs
two round trips and one commit `fsync`.

The mapping onto gameplay's ack-and-resend is the one the JSON store already relies on:

| Case | What happens |
| --- | --- |
| Commit succeeds, ack delivered | Gameplay drops the reward. |
| Commit succeeds, Medius dies before the ack | Gameplay resends; step 2 finds the receipt and returns the original outcome with `duplicate=true`; Medius acks. |
| Connection drops during `COMMIT` (uncertain) | Medius treats it as deferred; the resend either replays (it committed) or applies (it didn't). |
| Two worlds deliver the same `reward_id` | Can't normally happen (reward ids are bound to one reservation). If it does, the receipt's primary key admits one; the other replays, or gets `reward-receipt-mismatch` if the identity differs. |
| Database unreachable or failing over | Deferred; see [Outages](#outages). |

The other operations (create, loadout save/delete/select, skill purchase, nominations, Training,
set-level) follow the same pattern: lock the character row, apply the rule, write, commit. Create
relies on the unique indexes. A unique violation on `account` re-reads the row and returns
`existing-exact-profile` or `local-account-slot-occupied`, as today; one on `lower(name)` returns
`character-name-taken`. IDs come from `mag.character_ids`.

### Why there is no write-behind cache

A write-behind cache would acknowledge a reward before it is durable. A Medius crash or restart
would then lose XP that gameplay had already dropped from its outbox, and no receipt would exist
to catch it. Gameplay's in-memory outbox already is the write buffer: it holds every reward until
Medius confirms a durable commit. So Medius must commit before it acknowledges, with
`synchronous_commit` on, and the database is the only durable copy. **A Medius restart loses no
XP.** Unacknowledged rewards stay in gameplay and replay idempotently once Medius is back.
(Losing a gameplay worker still loses its outbox, as today; that is gameplay's limit, not the
store's.)

### Concurrency across worlds

In registry mode, the registry-wide `_progressionGate` serializes every reward commit and
admission read across all worlds. It also orders a reward commit against admission (`LocalDmeHost` around line 419), and
phase 1 keeps it unchanged, so behaviour matches the JSON backend exactly. With PostgreSQL the
gate is only needed per character. Phase 3 narrows it to a per-account gate if test-server
measurements show reward settlement waiting on it. The database's row lock already serializes
one character; the gate only has to keep Medius's own admission ordering.

### Optional per-batch commits

A match result releases up to one terminal reward per human, plus objective rows, in batches of
32. With one transaction per reward, a 256-player result costs 256 commits. At 2 to 5 ms each
that's about 0.5 to 1.3 s, serialized by the gate. A slow NAS could stretch it. The option, used
only if measurement shows it's needed:

- Medius applies one gameplay batch in one transaction, with a `SAVEPOINT` per reward.
- A refused reward rolls back to its savepoint, and the rest of the batch continues. Today a
  refusal stops the batch; this keeps the same stop point and commits what came before it.
- One `COMMIT` covers up to 32 rewards. Acknowledgements go out only after it.

The default stays one reward per transaction, because it is simpler and maps one-to-one onto
today's code.

### Read cache for logins

Login, MAPS, admission, matchmaking and party placement call `ReadForAccount`. Today each call
reads and parses the whole file. With PostgreSQL it is one indexed query for the character plus
its five loadout rows, well under a millisecond on the LAN, so **phase 1 has no cache.** If one
is ever needed, the `revision` column allows a cheap validity check
(`SELECT revision FROM mag.characters WHERE account = $1`), which also works across Medius
processes. Until then no cache can go stale. The extra `ReadForAccount` that Medius makes after
each reward, to notify the client, becomes a returned value from the same transaction.

### Outages

The store maps every Npgsql connection, timeout or server failure to a
`CharacterStoreUnavailableException`. It derives from `IOException`, so the existing handlers
apply unchanged:

- **Rewards defer, then land.** Medius logs `Gameplay reward persistence deferred ...
  reason=database unavailable` and stops the batch. Gameplay keeps the rewards and resends every
  second. When the database is back, each one commits once, or replays if its commit had in fact
  succeeded. No XP is lost or doubled.
- **Logins, creation, loadout and skill saves fail visibly.** The client gets the existing error
  result, and the log names the database as the reason. Nothing is acknowledged that isn't
  durable.
- **Timeouts are short,** so a dead database can't stall world ticks. The calls are synchronous,
  like the file store's, and run inside the settlement path. The settings are a 2 s connect
  timeout and a 3 s command timeout. After a failure, a circuit breaker fails calls immediately
  for 1 s (one resend period) rather than wait for another timeout. Every rejection is logged at
  most once per second.
- **CNPG failover.** Clients connect through the cluster's `-rw` Service. An in-flight
  transaction on the old primary fails and is deferred. Npgsql's pool discards broken
  connections, and the next attempt reaches the new primary. A planned switchover takes a few
  seconds.

One existing limit becomes more relevant. Gameplay's outbox holds at most 256 ordinary pending
rewards. Once it is full, gameplay refuses new reward-bearing events with `reward_outbox_full`,
and that refusal applies to the whole combat event. A database outage of more than about a minute
in a busy match would therefore start refusing kills. The JSON store has the same limit (its
outage is a full disk or lock), so this design doesn't make it worse. Two things keep outages
short: two instances and a switchover-first update method. The test plan measures failover time.
Sizing the outbox to admitted humans is a separate gameplay change, which this design recommends
before 128- or 256-player lobbies.

## Retention

Size no longer forces pruning, but receipts grow with play. A full 256-player world produces
tens of thousands of receipts per hour at about 400 bytes each, including indexes. Retention
replaces #301's and #305's compaction with one rule based on time:

- **A match row and all its receipts are pruned together** once its `last_reward_at` is older
  than `MAG_REWARD_LEDGER_RETENTION_DAYS`. The default is 7, valid from 1 to 90; startup fails
  naming the variable otherwise. Rows and receipts are never pruned apart. A pruned receipt
  whose match row remained would make a redelivered match result hit `match-reward-finalized`,
  which never succeeds, and that would wedge the resend loop.
- **Open and finalized rows age the same way.** #305's 24-hour TTL existed only to free space,
  so it doesn't apply to PostgreSQL. A redelivery would need gameplay to hold an unacknowledged
  reward in memory for seven days. #301's and #305's known limit, a reward credited twice if
  it stays unacknowledged longer than the TTL, therefore goes from "not expected" to effectively
  impossible.
- **History survives pruning.** In the same transaction that deletes a row, a summary goes into
  `mag.match_history`: XP, FRAGO points, finalized, result kind, receipt count and timestamps.
  `mag.character_matches` presents recent and pruned matches together. `match_history` is kept
  indefinitely. It holds one small row per character per match, about 40,000 a day with three
  full 256-player worlds playing all day. An optional `MAG_MATCH_HISTORY_RETENTION_DAYS` (unset
  means keep) can limit it later.
- **The prune runs in Medius, off the reward path.** It runs hourly, plus once at startup, in
  batches of 1,000 rows per transaction. It holds `pg_try_advisory_lock`, so only one process
  prunes; MAPS never does, and a second Medius would skip. Each run logs one line:
  `Character store ledger pruned: N match rows (F finalized, O open; R receipts), H history rows
  written`. Autovacuum handles the space.
- **Late rewards** behave as they do after #305's eviction. A reward for a pruned match opens a
  fresh row and is credited once. A match result for it is computed from that fresh row only,
  which is never more than the full bonus.
- **The JSON backend keeps #301 and #305 unchanged** while it exists.
  `MAG_REWARD_LEDGER_OPEN_MATCH_TTL_HOURS` applies only to it. With `postgres`, the variable is
  accepted and logged as unused, so a shared chart value doesn't break startup.

If receipt volume ever makes the hourly delete expensive, `reward_receipts` can be partitioned by
day. Its primary key would then include the partition column, so global `reward_id` uniqueness
would need a different scheme. At this scale batched deletes are enough, and partitioning is not
part of this design.

## Database placement

### Decision: a dedicated CNPG cluster per game environment

Recommended: a dedicated cluster in each game namespace. `mag-test-characters` serves the test
server (`mag-test`) first, then `mag-characters` serves live (`mag`). The alternative is a
`mag_characters` database inside the portal's `mag-portal-db`.

| | Dedicated cluster | Database in `mag-portal-db` |
| --- | --- | --- |
| Isolation of the game write path | Portal traffic, sign-in bursts, report uploads, vacuum and failovers can't slow reward commits, and the reverse. | One `shared_buffers`, one WAL, one `max_connections` (50) and one primary for both. A portal incident blocks match results, and a reward burst slows sign-in. |
| Reservations | Sized for the write path and reserved separately. | The portal's 250m/256Mi reservation was set for the portal alone after it was starved on 2026-11-24; it would have to grow for both. |
| Placement | Can require nodes other than `talos-2da-doc` (live game pods) and `nixos`. | The portal cluster runs one instance on `talos-2da-doc` today (`mag-portal-db-2`); game DB load would land on the live node unless the portal's placement changes too. |
| Backups and restores | Own backup chain and point-in-time recovery. Rolling characters back to before a bad release doesn't roll back Google links and name claims. | One chain; restoring either restores both. |
| Upgrades and maintenance | Independent PostgreSQL upgrades and restarts. | A portal maintenance restart also restarts the game DB. |
| Credentials | CNPG writes `mag-characters-app` in the game namespace, where Medius mounts it directly. No copying. | The app secret lives in `mag-portal`; it would have to be copied into `mag` with external-secrets or a re-sealed SealedSecret, and kept in sync with rotations. |
| Cost | Two more PostgreSQL pods per environment (about 250m CPU and 512Mi reserved each). | No new pods. |

The cost of the dedicated cluster is small. Every row in that table that matters to the game
write path favours it: isolation, reservations, placement and backups. The portal keeps reading
characters through Medius's admin API, as it does today, so neither side's schema leaks into the
other.

### Cluster settings

As in the portal's `database.yaml`, adjusted for the write path:

```yaml
apiVersion: postgresql.cnpg.io/v1
kind: Cluster
metadata: {name: mag-characters, namespace: mag}      # mag-test-characters in mag-test first
spec:
  instances: 2
  primaryUpdateMethod: switchover
  imageName: ghcr.io/cloudnative-pg/postgresql:18.1-system-trixie   # same major as the portal
  postgresql:
    parameters:
      shared_buffers: "128MB"
      max_connections: "50"
    synchronous: {method: any, number: 1, dataDurability: preferred}
  affinity:
    enablePodAntiAffinity: true
    topologyKey: kubernetes.io/hostname
    podAntiAffinityType: required                     # the two instances on different nodes
    nodeAffinity:
      requiredDuringSchedulingIgnoredDuringExecution:
        nodeSelectorTerms:
        - matchExpressions:
          - {key: kubernetes.io/hostname, operator: NotIn, values: [talos-2da-doc, nixos]}
  resources:
    requests: {cpu: 250m, memory: 512Mi}              # no CPU limit; no memory limit (as the portal)
  bootstrap:
    initdb: {database: mag, owner: mag, encoding: UTF8, localeCType: C, localeCollate: C}
  storage: {size: 10Gi, storageClass: synostorage}
  plugins:
  - name: barman-cloud.cloudnative-pg.io
    isWALArchiver: true
    parameters: {barmanObjectName: mag-characters-s3}
```

- **Placement.** Both instances stay off `talos-2da-doc` (live gameplay and Medius) and `nixos`
  (the user's gaming box, out of the cluster this week). The two instances run on different
  nodes. That leaves `talos-b8s-ysm` and the three control-plane nodes, which take workloads.
  Medius reaches the primary across the LAN, adding a fraction of a millisecond per round trip.
- **Synchronous replication, `dataDurability: preferred`.** A commit waits for the standby, so a
  failover after a lost primary loses no acknowledged reward. If the standby is down, commits
  continue on the primary alone rather than block. The cost is one more LAN round trip per
  commit, and the test plan measures it. Without it, a primary loss could drop rewards that
  Medius had already acknowledged; gameplay would not resend them, and the XP would be lost.
- **Reservations.** 250m CPU and 512Mi memory requested, with no CPU limit, following the
  portal's lesson that a BestEffort database loses its CPU to a CI runner. The Barman sidecar
  requests 50m/128Mi, as on the other clusters.
- **Storage.** `synostorage` is the Synology SAN, the same backend as today's `mag-accounts`
  volume, so commit latency is no worse than today's file `fsync`, and each commit writes far
  less. NAS load shows up as commit latency, which the test plan measures under load.
- **Connections.** The Npgsql pool in each process (main Medius, MAPS, `mag-admin`) has at most 8
  connections. No PgBouncer or CNPG `Pooler`: three processes don't need one. Reconsider for
  multi-Medius.

### Backups

The eight existing CNPG clusters use the Barman Cloud plugin. Each has an `ObjectStore` writing
to the AWS bucket `s3://cnpg-homelab-tekulvw` (us-west-2), with its own IAM identity, a
SealedSecret `cnpg-backup-s3`, continuous WAL archiving, a staggered daily base backup between
05:00 and 05:35 UTC and a 30-day recovery window. The homelab's `docs/cnpg/README.md` has the
procedure.

The character clusters follow it exactly. Each gets an `ObjectStore` at
`s3://cnpg-homelab-tekulvw/mag/mag-characters/` (and `mag-test/...`), its own IAM user and
sealed credentials, and a `ScheduledBackup` at 05:40 UTC with `target: prefer-standby`. Backups
leave the house and don't depend on the NAS, whose load disturbed RustFS today. A cluster counts
as protected only after its first backup completes and `scripts/cnpg-backups/check.py` passes.
The test-server cluster gets the same setup, so the restore drill runs on real backups.

`mag-portal-db` has no `ObjectStore` yet (checked 2026-11-25). That is outside this design, but
it should be onboarded the same way.

### Credentials

CNPG generates `<cluster>-app` in the cluster's namespace, with `host`, `port`, `dbname`, `user`,
`password` and `uri`. The chart mounts that Secret read-only into main Medius and MAPS, and Medius
builds its Npgsql connection from the files. Nothing is copied or re-sealed, and rotation is
CNPG's. The only hand-made secret is the backup S3 credential, sealed like the other clusters'.
Connections use TLS (`SSL Mode=VerifyFull` against the cluster's `<cluster>-ca` Secret; the chart mounts only its
`ca.crt` item, never `ca.key`). MAPS connects with `Options=-c default_transaction_read_only=on`, so its read-only
`/accounts` mount keeps an equivalent. A separate read-only role is unnecessary while the
portal reads through the admin API.

## Configuration

| Setting | Meaning |
| --- | --- |
| `MAG_CHARACTER_STORE` | `json` (default) or `postgres`. Anything else fails startup, naming the variable. |
| `MAG_CHARACTER_DB_SECRET_DIR` | Directory holding the CNPG app secret's files (`host`, `port`, `dbname`, `user`, `password`). Required with `postgres`. |
| `MAG_CHARACTER_DB_CA_FILE` | CA certificate for `VerifyFull`. Without it the connection uses `Require` and logs that the server isn't verified. |
| `MAG_REWARD_LEDGER_RETENTION_DAYS` | Postgres only. 1 to 90, default 7. |
| `MAG_MATCH_HISTORY_RETENTION_DAYS` | Postgres only, optional. Unset means keep. |

Chart values (defaults keep the JSON store and render nothing new):

```yaml
characterStore:
  backend: json               # json | postgres
  postgres:
    existingSecret: ""        # the CNPG <cluster>-app Secret; required with postgres
    caSecret: ""              # the CNPG <cluster>-ca Secret; optional
    ledgerRetentionDays: null
```

With `backend: json` the rendered manifests are byte-for-byte unchanged. The chart test asserts
that, and asserts that `postgres` without `existingSecret` fails to render. The `/accounts` volume
stays mounted in either mode. It holds the session markers and admin backups, and it keeps the
JSON file for rollback.

## Admin and portal reads

- `mag-admin accounts list`, `character show` and `character set-level` work unchanged on both
  backends through `ICharacterStore`. On PostgreSQL, `set-level --apply` writes the before and
  after rows to `mag.admin_changes` in the same transaction, instead of copying the whole file to
  `.maintenance-backups`.
- The portal keeps using Medius's HMAC admin API (`PORTAL_GAME_ADMIN_URL`). It already lists
  level, XP and skill points. A new read-only endpoint, `GET /v1/accounts/{account}/matches?
  limit=N`, returns rows from `mag.character_matches`, so the portal can show match history. On
  the JSON backend that endpoint returns only the store's current match rows.
  The portal never connects to the game database. That keeps the schema private to Medius, and
  it keeps the portal working the same whichever backend is configured.

## Migration

### Tools

All three run in the main Medius container, from the same image, as `mag-admin character-store`
subcommands:

- **`import --from /accounts/characters.json`**
  - Reads the file with the JSON backend's own reader, so every existing validation applies:
    schema 2 or 3, the catalog check of level against XP, and the ledger checks.
  - Refuses unknown fields and a target database that already has characters.
  - Writes everything in one transaction: characters, loadouts, match rows, receipts, and the
    ID sequence moved past the highest ID.
  - Undated match rows take the import time as `first_reward_at` and `last_reward_at`, so they
    get a full retention window. Receipts take the import time as `committed_at`.
  - Prints counts and a digest of the canonical projection (below), never player data.
- **`verify --json /accounts/characters.json`** builds a canonical projection of both stores and
  compares them. The projection covers each character's stored fields and its `ToRecord()`, each
  receipt with its outcome, and each match row. Match row order and timestamps are left out
  because they have no JSON equivalent. `verify` reports differences by account and field name,
  without values, and exits nonzero if there are any.
- **`export --to <path>`** writes a JSON store from PostgreSQL, using the JSON backend's writer
  and its #301/#305 compaction, so the result fits the 1 MiB bound. It refuses to overwrite an
  existing file unless given `--replace`, and then it keeps a timestamped copy. This is the
  rollback path.

### Why verification, not dual writes

Writing both stores on every operation would give the system two sources of truth. A failure
after one write and before the other leaves them disagreeing. Recovery would need a rule for
which one wins, and every reward would still pay for the full file rewrite that this design
removes. A one-time import, a field-by-field verify, and an export for rollback give the same
safety without that window.

### Order

1. **Merge the code with `json` as the default.** Live and test are unchanged; the synthetic
   suite runs against both backends in CI.
2. **Test server** (after the user reviews this design):
   1. Create `mag-test-characters` with backups, and wait for its first backup.
   2. With no matches running, stop Medius, copy `characters.json` aside, then `import` and
      `verify`.
   3. Switch the test release to `postgres` and play matches across all three modes.
   4. Exercise the failure cases: restart Medius mid-match, run a CNPG switchover mid-match,
      and delete the primary pod.
   5. Check that every reward lands exactly once: the receipt count matches gameplay's reward
      log, and no `reward-receipt-mismatch` appears.
   6. Measure commit latency (p50 and p99 per reward and per terminal burst) and failover time.
   7. Run `export` and `verify` against the export, then one rollback drill: switch to `json`
      with the exported file and confirm the characters.
   8. Run a backup restore drill into an isolated cluster, as `docs/cnpg/README.md` describes.
3. **Live**, as a separate user-approved rollout: the same steps in `mag`, in a window with no
   players, as releases already are. Keep the pre-import `characters.json` copy.
4. **One release later**, if live has been clean, a separate change removes the JSON backend's
   write path. A read-only JSON loader stays for `import`.

### Rollback

While the JSON backend exists:

1. Stop Medius and run `mag-admin character-store export --to /accounts/characters.json
   --replace`.
2. Set `characterStore.backend: json` and start Medius.

Progress earned on PostgreSQL carries over. Only receipts and match rows older than the JSON
store's compaction keeps are dropped, which #305 already treats as settled. Rolling back without
an export would revert every player to the pre-import file, so the runbook doesn't offer it.
Rolling back to an older Medius build works the same way, because older builds only know JSON.

## Test plan

All tests are synthetic and need no game files.

- **One suite, both backends.** About 35 Tests.MagPlugin checks construct `LocalCharacterStore`
  directly, among them the reward, FRAGO, leadership, Training, loadout preference
  (`LoadoutPreferenceStoreChecks`), nomination (`NominationPreferenceChecks`) and admin checks.
  They move to a store factory, and each runs once per backend. Where they read the JSON file,
  they go through the store API instead. The JSON-only compaction checks (#301, #305) stay
  JSON-only.
- **PostgreSQL-only checks:**
  - migrations apply on an empty database; a re-run is a no-op; a changed checksum and an
    unknown version are refused, naming the version;
  - concurrent rewards for one character from two connections credit XP once per `reward_id`;
  - the same `reward_id` delivered for two characters lets one commit and the other replay or
    mismatch;
  - concurrent creates with the same name produce one `character-name-taken`;
  - a dropped connection between `COMMIT` and its reply is replayed correctly (the test kills
    the backend with `pg_terminate_backend` from a second session at a hook point);
  - with the server stopped, `CharacterStoreUnavailableException` reaches the deferral path
    within the timeouts, and the breaker fails fast;
  - retention prunes rows and receipts together, writes history, and never prunes one without
    the other; a late reward after pruning is credited once;
  - `import` → `verify` → `export` → JSON round-trip on generated stores, including schema 2
    files, undated rows, leadership floors and null-meaning fields; `import` refuses unknown
    fields and a non-empty database.
- **CI.** Horizon's checks run in the Docker `check` stage of `horizon/Dockerfile`. That stage
  installs Debian's PostgreSQL server packages and starts a throwaway cluster as a non-root user
  inside the same `RUN`, with `fsync=off` and a socket in `/tmp`, before
  `dotnet run --project Tests.MagPlugin`. `MAG_TEST_POSTGRES` tells the suite where it is.
  Running it in the stage keeps it with the image build that already gates every Medius image,
  and it needs no host networking from buildx.
  - The suite fails rather than skips when `MAG_TEST_POSTGRES` is missing in CI
    (`CI=true`). Locally, without it, the PostgreSQL half reports itself as not run.
  - If the stage approach proves too slow, the fallback is the portal job's pattern: start a
    throwaway server on the runner, where `arc-runner-mag` already ships PostgreSQL 16, and run
    the published test image with host networking.
  - The SQL uses nothing newer than PostgreSQL 14, so CI's 16 or Debian's 17 cover the cluster's
    18. The test server is the acceptance check on 18.
- **Chart.** `tests/test_helm_chart.py` checks four things:
  - with `json`, the manifests are unchanged;
  - with `postgres`, the Secret mounts and environment are present in main Medius and MAPS only;
  - `postgres` without a Secret fails to render;
  - the retention value's bounds.
- **Test server acceptance.** The steps in [Order](#order), step 2, with these numbers recorded
  in the PR:
  - commit latency;
  - failover time;
  - reward exactly-once counts;
  - prune duration.

## Risks

| Risk | Mitigation |
| --- | --- |
| A rule changes subtly in the split | The rules move rather than get rewritten; one suite runs against both backends; `verify` compares `ToRecord()` projections after import. |
| Lost acknowledged rewards on primary loss | Synchronous replication with `dataDurability: preferred`; switchover-first updates. |
| Outage longer than gameplay's outbox | Two instances, short timeouts, circuit breaker; failover measured; recommend sizing the gameplay outbox before large lobbies (separate change). |
| NAS latency spikes slow commits | Measured on the test server under load; optional per-batch commits; same storage as today with far smaller writes. |
| Schema drift between builds | Checksummed migrations; downgrade refused; rollback through `export`, never an old build on a new schema. |
| Secrets or player data in logs | Connection settings are read from files and never logged; tools print counts, digests and field names only. |
| Import or verify run against the wrong environment | The commands print the target host and database name, and refuse a non-empty target. The runbook stops Medius first. |
| Backups that don't restore | Restore drill on the test server before live; `check.py` passing is the gate. |

## SQLite, and why not

SQLite on the `/accounts` volume would remove the size cap and the whole-file rewrites with no new
service. It would also give transactions and unique constraints. Its limits rule it out here:

- **Single writer, one node.** SQLite serializes all writers on one database lock. Its file must
  sit on a volume one pod mounts (RWO `synostorage`), so multi-Medius would need a different
  store anyway.
- **No portal reads without Medius.** Only processes in the same pod can open the file safely.
  That's true of the admin API approach too, but SQLite rules out every other option.
- **Backups and failover by hand.** There's no WAL archiving to object storage or point-in-time
  recovery without extra tooling such as Litestream. A failover means rescheduling the pod and
  re-attaching the volume, minutes of downtime that would fill gameplay's outbox. CNPG already
  provides all of this, and the cluster runs eight such databases.
- **The same storage path.** WAL `fsync` on the same SAN volume, so commit latency would be no
  better than PostgreSQL's.

SQLite would be a reasonable choice for a single-box lab server. The JSON backend already covers
that case, and it stays available.

## Phases

1. **This design** (docs only).
2. **Refactor, no behaviour change.**
   - Extract `CharacterRules` and `ICharacterStore`; `JsonCharacterStore` is today's code.
   - Move the tests to the factory.
   - CI proves the JSON path is unchanged.
3. **PostgreSQL backend.**
   - Npgsql, migrations, `PostgresCharacterStore`, retention, the outage mapping and breaker.
   - The suite against a throwaway PostgreSQL in CI.
4. **Tools.** `mag-admin character-store migrate|import|verify|export`, the admin-changes audit,
   and the matches endpoint.
5. **Chart.** The `characterStore` values, Secret mounts and chart tests.
6. **Homelab** (draft, not applied). `mag-test-characters` and `mag-characters` with
   reservations, placement, synchronous replication, ObjectStores, ScheduledBackups and sealed
   backup credentials.
7. **Test server cutover**, after review. Then live, as a separate approval.
8. **One release later.** Remove the JSON write path and narrow `_progressionGate` to per-account
   if measurements call for it.
