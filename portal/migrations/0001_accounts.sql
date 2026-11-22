-- Player accounts, their launcher devices, and the short-lived state of sign-in and linking.

CREATE TABLE accounts (
    -- Fits the game server's unsigned 32-bit account ID.
    id integer GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    provider text NOT NULL,
    subject text NOT NULL,
    email text,
    player_name text,
    created_at timestamptz NOT NULL DEFAULT now(),
    banned_at timestamptz,
    ban_reason text,
    UNIQUE (provider, subject)
);

-- The game server keys characters by lowercase name.
CREATE UNIQUE INDEX accounts_player_name_key ON accounts (lower(player_name));

-- Names nobody may claim: imported existing names awaiting their owners, and names of
-- deleted accounts (characters on the game server stay keyed by the name).
CREATE TABLE reserved_names (
    name_key text PRIMARY KEY CHECK (name_key = lower(name_key)),
    name text NOT NULL,
    reason text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE login_flows (
    state text PRIMARY KEY,
    nonce text NOT NULL,
    pkce_verifier text NOT NULL,
    return_to text NOT NULL,
    expires_at timestamptz NOT NULL
);

CREATE TABLE web_sessions (
    id_hash bytea PRIMARY KEY,
    account_id integer NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    csrf_token text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL
);
CREATE INDEX web_sessions_account ON web_sessions (account_id);

CREATE TABLE device_codes (
    device_code_hash bytea PRIMARY KEY,
    user_code text NOT NULL UNIQUE,
    client_label text NOT NULL,
    status text NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'approved', 'denied', 'consumed')),
    account_id integer REFERENCES accounts (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    last_poll_at timestamptz
);

CREATE TABLE devices (
    id integer GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    account_id integer NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    token_hash bytea NOT NULL UNIQUE,
    label text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    last_used_at timestamptz
);
CREATE INDEX devices_account ON devices (account_id);
