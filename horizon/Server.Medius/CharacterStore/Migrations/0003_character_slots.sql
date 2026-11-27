-- Up to three characters per account, in slots 0 to 2, and soft deletion.
-- Retail MAG gave each account one character slot and, from August 2010, two more. Before this
-- migration an account had at most one character; each existing character becomes slot 0.
-- A character the player deletes keeps its row (deleted_at is set) but frees its slot, and its name
-- is moved aside (name~id, the original in original_name), because the client names characters after
-- their slot. Operators restore or purge it with mag-admin. Only live characters hold a slot.
-- This migration is one-way: an older build refuses a schema newer than its own.

-- Voice sanctions belong to the account, not to one character, and outlive a purged character.
ALTER TABLE mag.voice_sanctions DROP CONSTRAINT voice_sanctions_account_fkey;
ALTER TABLE mag.voice_sanctions ADD CONSTRAINT voice_sanctions_account_check
  CHECK (account ~ '^[a-z0-9_-]{3,16}$');

ALTER TABLE mag.characters DROP CONSTRAINT characters_account_key;
ALTER TABLE mag.characters
  ADD COLUMN slot          smallint NOT NULL DEFAULT 0 CHECK (slot BETWEEN 0 AND 2),
  ADD COLUMN deleted_at    timestamptz,
  ADD COLUMN original_name text CHECK (original_name ~ '^[!-~]{1,32}$'),
  ADD CHECK (original_name IS NULL OR deleted_at IS NOT NULL);
CREATE UNIQUE INDEX characters_account_slot_key ON mag.characters (account, slot) WHERE deleted_at IS NULL;
CREATE INDEX characters_account ON mag.characters (account);

-- The character an operator or player change applied to; null for changes recorded before slots.
ALTER TABLE mag.admin_changes ADD COLUMN character_id bigint;
