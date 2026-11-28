-- Public names may change; game character ownership never follows those changes.
ALTER TABLE accounts ADD COLUMN game_account_name text;
ALTER TABLE accounts ADD COLUMN name_changed_at timestamptz;
UPDATE accounts SET game_account_name = lower(player_name) WHERE player_name IS NOT NULL;
ALTER TABLE accounts ADD CONSTRAINT accounts_game_account_name_check CHECK (
    (player_name IS NULL AND game_account_name IS NULL) OR
    (player_name IS NOT NULL AND game_account_name IS NOT NULL AND game_account_name ~ '^[a-z][a-z0-9_-]{2,14}$')
);
CREATE UNIQUE INDEX accounts_game_account_name_key ON accounts(game_account_name);

-- Detached reservations can only be recovered explicitly by an administrator. Live
-- historical names belong to their account and cannot be released or transferred.
ALTER TABLE reserved_names ADD COLUMN owner_account_id integer REFERENCES accounts(id) ON DELETE SET NULL;
ALTER TABLE reserved_names ADD COLUMN game_account_name text;
ALTER TABLE reserved_names ADD COLUMN recovery_only boolean NOT NULL DEFAULT false;
UPDATE reserved_names SET game_account_name = name_key;
-- Legacy reservations include deleted owners, without a typed distinction. Keep
-- explicit assignment/recovery available but never turn them into public claims.
UPDATE reserved_names SET recovery_only = true;
CREATE INDEX reserved_names_owner ON reserved_names(owner_account_id);

CREATE FUNCTION preserve_game_account_name() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'UPDATE' AND OLD.game_account_name IS NOT NULL AND
       NEW.game_account_name IS DISTINCT FROM OLD.game_account_name THEN
        RAISE EXCEPTION 'game account owner is immutable' USING ERRCODE = '23514';
    END IF;
    IF NEW.player_name IS NOT NULL AND NEW.game_account_name IS NULL THEN
        NEW.game_account_name := lower(NEW.player_name);
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER accounts_immutable_game_owner BEFORE INSERT OR UPDATE ON accounts
    FOR EACH ROW EXECUTE FUNCTION preserve_game_account_name();

-- Reports retain the spelling at filing time while their target key stays the
-- immutable game owner. Existing reports already have the correct owner key.
ALTER TABLE player_reports DROP CONSTRAINT player_reports_check;
ALTER TABLE player_reports ADD CONSTRAINT player_reports_target_name_check
    CHECK (char_length(target_name) BETWEEN 1 AND 32);
