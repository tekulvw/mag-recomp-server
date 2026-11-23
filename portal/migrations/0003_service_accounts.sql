-- Service accounts: fixed player identities for automation (test clients), with launcher
-- tokens that staff mint instead of a Google sign-in. Their provider is 'service' and their
-- subject random, so no Google sign-in can ever match one, and they always hold a name.

ALTER TABLE accounts
    ADD COLUMN kind text NOT NULL DEFAULT 'person' CHECK (kind IN ('person', 'service'));

ALTER TABLE accounts ADD CONSTRAINT accounts_kind_shape CHECK (
    (kind = 'person' AND provider <> 'service')
    OR (kind = 'service' AND provider = 'service' AND email IS NULL AND player_name IS NOT NULL)
);
