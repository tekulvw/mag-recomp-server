-- Launchers report network gaps: the game server went silent during a match while the client's
-- frames kept coming (docs/portal-reports.md). Widens the trigger check by one value.

ALTER TABLE reports DROP CONSTRAINT reports_trigger_check;
ALTER TABLE reports ADD CONSTRAINT reports_trigger_check
    CHECK (trigger IN ('crash', 'launch_error', 'halt', 'slow_frames', 'audio', 'network', 'manual'));
