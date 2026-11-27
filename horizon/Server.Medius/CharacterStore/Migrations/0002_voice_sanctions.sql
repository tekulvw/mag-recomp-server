-- Server-side voice moderation: at most one mute and one ban per account, each until expires_at or,
-- when that is null, until lifted. Design: docs/superpowers/specs/2026-11-25-voice-relay-design.md
-- ("Moderation"). Changes are journalled in mag.admin_changes (operations voice-mute, voice-ban,
-- voice-unmute and voice-unban); a lifted sanction leaves this table.
CREATE TABLE mag.voice_sanctions (
  account    text NOT NULL REFERENCES mag.characters (account) ON DELETE CASCADE,
  kind       text NOT NULL CHECK (kind IN ('mute', 'ban')),
  reason     text CHECK (reason ~ '^[ -~]*$' AND length(reason) <= 256),
  operator   text NOT NULL CHECK (operator ~ '^[ -~]{1,254}$'),
  created_at timestamptz NOT NULL,
  expires_at timestamptz CHECK (expires_at > created_at),
  PRIMARY KEY (account, kind)
);
