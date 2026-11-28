-- Durable mappings use verified provider subjects, never display-name matching.
CREATE TABLE external_identities (
    provider text NOT NULL CHECK (provider = 'psn'),
    subject text NOT NULL,
    account_id integer NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    online_id text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (provider, subject),
    UNIQUE (account_id, provider)
);

-- Only Medius creates these, after verifying a Sony ticket. Raw codes/tickets are never stored.
CREATE TABLE console_link_proofs (
    code_hash bytea PRIMARY KEY CHECK (octet_length(code_hash) = 32),
    provider text NOT NULL CHECK (provider = 'psn'),
    subject text NOT NULL UNIQUE,
    online_id text NOT NULL,
    expires_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX console_link_proofs_expiry ON console_link_proofs (expires_at);
