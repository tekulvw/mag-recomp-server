-- Requested console names are owner-scoped aliases. Deleted rows retain history but reserve no alias.
ALTER TABLE mag.characters ADD COLUMN console_requested_name text
  CHECK (console_requested_name ~ '^[!-~]{1,32}$');
CREATE UNIQUE INDEX characters_live_console_alias_key
  ON mag.characters (account, lower(console_requested_name))
  WHERE deleted_at IS NULL AND console_requested_name IS NOT NULL;
