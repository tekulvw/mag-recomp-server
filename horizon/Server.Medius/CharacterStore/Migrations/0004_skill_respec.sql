-- Skill respecs: how many a character has taken and its respec point balance.
-- Retail's character record carries both (RespecCount, a byte, and RespecPoints, a uint32); a respec
-- costs the authored fee for the current count and refunds every owned skill. Both columns are
-- nullable and no row is rewritten: null reads as 0, as every character had before this migration.
-- An operator floor (MAG_RESPEC_POINTS_FLOOR) is applied on read and is never stored here.
-- This migration is one-way: an older build refuses a schema newer than its own.
ALTER TABLE mag.characters
  ADD COLUMN respec_count  smallint CHECK (respec_count BETWEEN 0 AND 255),
  ADD COLUMN respec_points bigint CHECK (respec_points BETWEEN 0 AND 4294967295);
