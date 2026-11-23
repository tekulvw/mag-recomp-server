-- Performance and error reports that launchers send (docs/portal-reports.md). Only metadata and
-- small JSON documents live here; the gzip'd logs live in the report store (S3 or a folder),
-- keyed by the report's date, ID and log name. No client address is kept.

CREATE TABLE reports (
    -- The launcher's own UUID for the report, so a resend is recognised.
    id uuid PRIMARY KEY,
    account_id integer NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    received_at timestamptz NOT NULL DEFAULT now(),
    trigger text NOT NULL
        CHECK (trigger IN ('crash', 'launch_error', 'halt', 'slow_frames', 'audio', 'manual')),
    platform text NOT NULL CHECK (platform IN ('macos', 'linux', 'windows')),
    client_version text NOT NULL,
    client jsonb NOT NULL,
    session jsonb,
    metrics jsonb,
    log_names text[] NOT NULL DEFAULT '{}',
    -- Total stored (compressed) size of the logs.
    log_bytes bigint NOT NULL DEFAULT 0
);
CREATE INDEX reports_account_received ON reports (account_id, received_at);
CREATE INDEX reports_received ON reports (received_at);
