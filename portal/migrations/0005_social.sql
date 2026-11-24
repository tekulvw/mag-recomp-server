-- Friends, friend requests, blocks and the game presence relay (docs/portal-social.md). Every
-- row goes with either account when it is deleted.

-- One row per mutual friendship, lower account ID first.
CREATE TABLE friendships (
    account_lo integer NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    account_hi integer NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (account_lo, account_hi),
    CHECK (account_lo < account_hi)
);
CREATE INDEX friendships_hi ON friendships (account_hi);

CREATE TABLE friend_requests (
    from_account integer NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    to_account integer NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (from_account, to_account),
    CHECK (from_account <> to_account)
);
CREATE INDEX friend_requests_to ON friend_requests (to_account);

CREATE TABLE blocks (
    blocker integer NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    blocked integer NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (blocker, blocked),
    CHECK (blocker <> blocked)
);
CREATE INDEX blocks_blocked ON blocks (blocked);

-- The latest presence a player's game published, opaque to the portal. Going offline deletes
-- the row; a row older than the presence lifetime (60 s) counts as offline and is purged.
CREATE TABLE presence (
    account_id integer PRIMARY KEY REFERENCES accounts (id) ON DELETE CASCADE,
    data bytea NOT NULL CHECK (octet_length(data) <= 1024),
    updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX presence_updated ON presence (updated_at);
