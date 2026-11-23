-- Site staff and what they did. Owners come from configuration (PORTAL_OWNER_EMAILS) and are
-- never stored here, so the site cannot lock its owner out.

CREATE TABLE staff (
    email_key text PRIMARY KEY CHECK (email_key = lower(email_key)),
    role text NOT NULL CHECK (role IN ('admin', 'moderator')),
    granted_by integer REFERENCES accounts (id) ON DELETE SET NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE admin_log (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    actor_account integer REFERENCES accounts (id) ON DELETE SET NULL,
    actor_email text NOT NULL,
    action text NOT NULL,
    target text NOT NULL,
    detail text NOT NULL DEFAULT '',
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX admin_log_created ON admin_log (created_at DESC);
