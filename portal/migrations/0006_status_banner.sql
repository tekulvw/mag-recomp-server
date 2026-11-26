-- The maintenance or incident notice staff set from the admin panel's status page. It shows on
-- the public status page and the home page. One row at most.

CREATE TABLE status_banner (
    id smallint PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    message text NOT NULL CHECK (char_length(message) BETWEEN 1 AND 500),
    restart_expected_at timestamptz,
    updated_by text NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now()
);
