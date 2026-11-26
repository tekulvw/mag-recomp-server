-- Characters, reward receipts and match progress for the PostgreSQL character store.
-- Design: docs/superpowers/specs/2026-11-25-postgres-character-store-design.md.
-- Protocol identifiers are uint32 and stored as bigint with range checks; byte fields are smallint.

CREATE TABLE mag.characters (
  id                         bigint PRIMARY KEY CHECK (id BETWEEN 1 AND 4294967295),
  account                    text NOT NULL UNIQUE,
  name                       text NOT NULL CHECK (name ~ '^[!-~]{1,32}$'),
  faction                    smallint NOT NULL CHECK (faction BETWEEN 0 AND 2),
  head_id                    smallint NOT NULL CHECK (head_id BETWEEN 0 AND 255),
  voice_id                   smallint NOT NULL CHECK (voice_id BETWEEN 0 AND 255),
  flags                      smallint NOT NULL DEFAULT 0 CHECK (flags BETWEEN 0 AND 255),
  universe_id                bigint NOT NULL CHECK (universe_id BETWEEN 0 AND 4294967295),
  np_id                      bytea NOT NULL CHECK (octet_length(np_id) = 36),
  experience                 integer CHECK (experience >= 0),
  level                      smallint CHECK (level BETWEEN 1 AND 255),
  skill_points               smallint CHECK (skill_points BETWEEN 0 AND 255),
  owned_skills               bigint[] CHECK (cardinality(owned_skills) = 5),
  unlocked_items             bigint[] CHECK (cardinality(unlocked_items) = 24),
  last_used_loadout          smallint NOT NULL DEFAULT 0 CHECK (last_used_loadout BETWEEN 0 AND 4),
  loadout_layout_revision    numeric(20,0) NOT NULL DEFAULT 0 CHECK (loadout_layout_revision BETWEEN 0 AND 18446744073709551615),
  leadership_level           smallint CHECK (leadership_level BETWEEN 0 AND 255),
  nominations                smallint CHECK (nominations BETWEEN 0 AND 255),
  matches_since_last_command smallint CHECK (matches_since_last_command BETWEEN 0 AND 255),
  leadership_points          bigint CHECK (leadership_points BETWEEN 0 AND 4294967295),
  leadership_level_floor     smallint CHECK (leadership_level_floor BETWEEN 0 AND 255),
  revision                   bigint NOT NULL DEFAULT 1,
  created_at                 timestamptz NOT NULL DEFAULT now(),
  updated_at                 timestamptz NOT NULL DEFAULT now(),
  -- A leadership record is either absent (all null) or present (level, nominations and recency set).
  CHECK ((leadership_level IS NULL) = (nominations IS NULL)
     AND (nominations IS NULL) = (matches_since_last_command IS NULL)),
  CHECK (leadership_points IS NULL OR leadership_level IS NOT NULL),
  CHECK (leadership_level_floor IS NULL OR
         (leadership_points IS NOT NULL AND leadership_level_floor = leadership_level))
);
-- Names are printable ASCII, so lower() is locale independent.
CREATE UNIQUE INDEX characters_name_key ON mag.characters (lower(name));
CREATE SEQUENCE mag.character_ids AS bigint MINVALUE 1 MAXVALUE 4294967295 OWNED BY mag.characters.id;

-- Saved kits: no rows until the first save, then exactly five.
CREATE TABLE mag.character_loadouts (
  character_id bigint   NOT NULL REFERENCES mag.characters ON DELETE CASCADE,
  slot         smallint NOT NULL CHECK (slot BETWEEN 0 AND 4),
  update_id    smallint NOT NULL CHECK (update_id BETWEEN 0 AND 255),
  items        smallint[] NOT NULL CHECK (cardinality(items) = 22),
  PRIMARY KEY (character_id, slot)
);

-- One character's progress in one match: the basis for that match's later rewards.
CREATE TABLE mag.match_progress (
  character_id    bigint  NOT NULL REFERENCES mag.characters ON DELETE CASCADE,
  match_key       text    NOT NULL CHECK (match_key ~ '^[!-~]+$' AND length(match_key) <= 256),
  experience      integer NOT NULL CHECK (experience >= 0),
  fraction        real    NOT NULL CHECK (fraction >= 0 AND fraction < 1),
  frago_points    integer CHECK (frago_points >= 0 AND frago_points <= experience),
  finalized       boolean NOT NULL DEFAULT false,
  result_kind     text,
  first_reward_at timestamptz NOT NULL,
  last_reward_at  timestamptz NOT NULL,
  PRIMARY KEY (character_id, match_key)
);
CREATE INDEX match_progress_last_reward ON mag.match_progress (last_reward_at);

-- Immutable replay receipts. The global reward id makes redelivery idempotent.
CREATE TABLE mag.reward_receipts (
  reward_id          text    PRIMARY KEY CHECK (reward_id ~ '^[!-~]+$' AND length(reward_id) <= 256),
  character_id       bigint  NOT NULL,
  match_key          text    NOT NULL,
  kind               text    NOT NULL,
  units              integer NOT NULL,
  leadership         jsonb,
  frago              boolean,
  outcome            jsonb   NOT NULL,
  experience_awarded integer GENERATED ALWAYS AS ((outcome->>'experienceAwarded')::integer) STORED,
  committed_at       timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (character_id, match_key) REFERENCES mag.match_progress ON DELETE CASCADE
);
CREATE INDEX reward_receipts_row ON mag.reward_receipts (character_id, match_key);

-- Summaries of pruned match rows, kept for match history.
CREATE TABLE mag.match_history (
  character_id    bigint  NOT NULL REFERENCES mag.characters ON DELETE CASCADE,
  match_key       text    NOT NULL,
  experience      integer NOT NULL,
  frago_points    integer,
  finalized       boolean NOT NULL,
  result_kind     text,
  rewards         integer NOT NULL,
  first_reward_at timestamptz NOT NULL,
  last_reward_at  timestamptz NOT NULL,
  PRIMARY KEY (character_id, match_key)
);

CREATE VIEW mag.character_matches AS
  SELECT p.character_id, p.match_key, p.experience, p.frago_points, p.finalized, p.result_kind,
         (SELECT count(*) FROM mag.reward_receipts r
           WHERE r.character_id = p.character_id AND r.match_key = p.match_key)::integer AS rewards,
         p.first_reward_at, p.last_reward_at
    FROM mag.match_progress p
  UNION ALL
  SELECT character_id, match_key, experience, frago_points, finalized, result_kind, rewards,
         first_reward_at, last_reward_at
    FROM mag.match_history;

-- Operator changes, with the character row before and after.
CREATE TABLE mag.admin_changes (
  id        bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  at        timestamptz NOT NULL DEFAULT now(),
  operation text  NOT NULL,
  account   text  NOT NULL,
  before    jsonb NOT NULL,
  after     jsonb NOT NULL
);
