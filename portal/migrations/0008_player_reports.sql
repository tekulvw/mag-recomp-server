-- Reports players file about other players' conduct, for the moderators' queue
-- (docs/portal-player-reports.md). A report names a character (the game server keys characters
-- by lowercase player name); nothing is attached automatically and nothing happens to the
-- reported player until a moderator acts through the existing admin tools.
--
-- Deleting the reporter's account keeps the report without them (reporter_account becomes
-- NULL). Deleting the reported player's account keeps the report on the character, whose name
-- stays reserved (target_account becomes NULL).

CREATE TABLE player_reports (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    reporter_account integer REFERENCES accounts (id) ON DELETE SET NULL,
    -- The character reported: its game server key and its spelling when reported.
    target_key text NOT NULL CHECK (target_key = lower(target_key) AND char_length(target_key) BETWEEN 1 AND 32),
    target_name text NOT NULL CHECK (lower(target_name) = target_key),
    -- The portal account holding that name when reported, if any.
    target_account integer REFERENCES accounts (id) ON DELETE SET NULL,
    reason text NOT NULL
        CHECK (reason IN ('cheating', 'harassment', 'griefing', 'offensive_name', 'other')),
    description text NOT NULL CHECK (char_length(description) BETWEEN 1 AND 1000),
    -- The reporter's approximate time of the incident, if they gave one.
    occurred_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    status text NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'actioned', 'dismissed')),
    -- The latest review; the audit log (admin_log) keeps every change.
    reviewed_by integer REFERENCES accounts (id) ON DELETE SET NULL,
    reviewed_by_email text,
    review_note text CHECK (char_length(review_note) <= 1000),
    reviewed_at timestamptz,
    CHECK (reporter_account IS NULL OR target_account IS NULL OR reporter_account <> target_account)
);

-- The queue: by status, newest first.
CREATE INDEX player_reports_status_created ON player_reports (status, created_at DESC);
-- Other reports about the same character.
CREATE INDEX player_reports_target ON player_reports (target_key, status);
-- A reporter's list and their daily allowance.
CREATE INDEX player_reports_reporter_created ON player_reports (reporter_account, created_at DESC);
-- One open report per reporter and character: a second one is refused.
CREATE UNIQUE INDEX player_reports_one_open ON player_reports (reporter_account, target_key)
    WHERE status = 'open' AND reporter_account IS NOT NULL;
