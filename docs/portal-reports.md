# Launcher performance and error reports

The MAG launchers (macOS, Linux and Windows) report frame-rate dips, slow frames, audio timing
problems, crashes and launch errors to the accounts portal, with gzip'd logs. Reporting is on by
default in the launcher and can be turned off in MAG Settings. This page is the server contract,
its limits, where reports are kept and how staff read them. The portal's
[README](../portal/README.md) covers the rest of the portal.

## Sending a report

`POST /api/v1/reports` with `Authorization: Bearer <device token>` (the launcher's `mgd_` token,
as for tickets) and `Content-Type: application/json`. The body is at most 4 MiB.

```json
{
  "schema_version": 1,
  "report_id": "0b9c4f5e-1d2a-4b3c-8d4e-5f6a7b8c9d0e",
  "trigger": "slow_frames",
  "client": { "version": "1.4.0", "platform": "linux", "gpu": "…" },
  "session": { "start": "2026-11-23T18:00:00Z", "end": "2026-11-23T18:42:10Z", "server": "play.magtest.online", "exit_code": 0, "duration_s": 2530 },
  "metrics": { "frame_ms": { "p50": 16.6, "p95": 24.1, "p99": 48.0 }, "slow_frames": 112, "hitches": 3, "audio": { "underruns": 0 } },
  "logs": { "launcher.log": { "encoding": "gzip+base64", "data": "H4sIAAAAAAAA…" } }
}
```

| Field | Rule |
| --- | --- |
| `schema_version` | `1`. |
| `report_id` | A UUID version 4 the launcher generates; the idempotency key. Any case; stored lowercase. |
| `trigger` | `crash`, `launch_error`, `halt`, `slow_frames`, `audio` or `manual`. |
| `client` | Required JSON object with `version` (a string of 1 to 64 characters) and `platform` (`macos`, `linux` or `windows`); any other fields are free-form. At most 16 KiB serialized. |
| `session` | Optional JSON object (or `null`), free-form, such as `start`, `end` (RFC 3339), `server`, `exit_code`, `duration_s` and `user_comment` (below). At most 12 KiB serialized. |
| `metrics` | Optional JSON object (or `null`), free-form, such as a frame-time histogram, slow-frame counts, p50/p95/p99, hitches, audio underruns and drift. At most 64 KiB serialized. |
| `logs` | Optional map of log name to `{"encoding": "gzip+base64", "data": "<base64>"}`. Names match `^[a-z0-9][a-z0-9._-]{0,63}$`; at most 8 logs; each decoded log at most 2 MiB and all of them together at most 3 MiB; each must start with the gzip magic bytes `1f 8b`. |

`session.user_comment` is the player's own words on a `manual` report: a UTF-8 string the launcher
has already redacted, at most 1000 characters (Unicode scalar values). A longer one is refused with
`400 invalid_report` and the detail `user_comment too long`, checked before the size of `session`.
The 12 KiB cap on `session` holds the longest comment in any characters (at most 6002 bytes
serialized, when every character is escaped as `\u00XX`) with more than 4 KiB left for the other
fields. Staff tools show a comment only when it is a string with something besides white space in
it; any other value is kept as free-form JSON and shown only in the session JSON.

Any other top-level field, or another field inside a log entry, is refused. `client`, `session` and
`metrics` are stored as given (JSONB), with sizes measured on their serialized form. Logs are stored
compressed, exactly as sent; the portal never inflates them on receipt.

### Responses

All bodies are JSON.

| Status | Body | Meaning |
| --- | --- | --- |
| 201 | `{"report_id": "…", "received_at": "<RFC 3339>"}` | Stored. |
| 200 | the same | This account already sent this `report_id`; nothing new was stored and it does not count against the allowance. `received_at` is the original time. |
| 400 | `{"error": "invalid_report", "detail": "<short reason>"}` | The body broke the rules above. Do not resend it unchanged. |
| 401 | `{"error": "invalid_token"}` | Missing, unknown or revoked token, as for tickets. |
| 403 | `{"error": "banned"}` | The account is banned. |
| 409 | `{"error": "report_id_conflict"}` | Another account holds this `report_id`; generate a new one. |
| 413 | `{"error": "too_large"}` | The body is over 4 MiB. |
| 429 | `{"error": "rate_limited"}` with `Retry-After: <seconds>` | Over the account's allowance, or over 60 report requests a minute from one address. Keep the report and retry after that many seconds. |
| 500 | `{"error": "server_error"}` | The report could not be stored (the log store failed). Retry later with the same `report_id`. |
| 503 | `{"error": "reports_disabled"}` | The server does not take reports (`PORTAL_REPORTS=off`). |

The body size is checked first, then the switch, the per-address limit, the token, the ban, the
body's contents, the `report_id` and finally the account's allowance.

### Allowance

Each account may store 20 reports in any rolling 24 hours (service accounts: 200). Resends of a
stored report do not count. `Retry-After` is the time until the oldest report in the window leaves
it.

## Storage and retention

Rows live in the `reports` table (migration `0004_reports.sql`): the report ID, account, time
received, trigger, platform, client version, the `client`, `session` and `metrics` JSON, the log
names and their total compressed size. No client address is kept. The rows are small; logs never
go into PostgreSQL.

Logs live in the report store, under `<yyyy-mm-dd>/<report_id>/<name>.gz` (the UTC day the report
arrived):

- **`s3`** (production): objects `reports/<yyyy-mm-dd>/<report_id>/<name>.gz` in a private
  S3-compatible bucket (RustFS), with path-style addressing and SigV4 presigned requests,
  uploaded with `Content-Type: application/gzip`. At start the portal checks the bucket with a
  `HEAD` request and refuses to start if it is missing or refuses the keys; errors never include
  the keys or a signed URL. Give the bucket an expiry lifecycle rule a little longer than the
  retention (31 days for the default 30) as a backstop for objects nothing refers to.
- **`dir`** (tests and local development): files under `PORTAL_REPORT_DIR` with the same layout,
  folders mode 0700 and files 0600, each written to a temporary file, flushed and renamed into
  place. The folder must exist and be writable at start.

The logs are written first and the row second, so a row never points at a missing log; if the row
cannot be written, the logs just uploaded are removed. While a report is being stored the portal
holds database locks on its ID and account, so a resend racing the original, or reports racing the
allowance, wait their turn.

Every hour `mag-portal serve` deletes reports older than `PORTAL_REPORT_RETENTION_DAYS` (default
30): their logs, then their rows. With the `dir` store it also sweeps logs no row refers to (an hour
after they were written, so reports being stored are safe) and whole days before the cutoff; the
`s3` store relies on the bucket's lifecycle rule for those. Deleting an account (by its owner or by
staff) removes its reports' logs and, with the account, their rows.

### Configuration

| Variable | Default | Meaning |
| --- | --- | --- |
| `PORTAL_REPORTS` | `on` | `on` takes reports and needs a working store at start; `off` answers `503 reports_disabled` and needs no store. |
| `PORTAL_REPORT_STORE` | — | `s3` or `dir`; required when reports are on. |
| `PORTAL_REPORT_S3_ENDPOINT` | — | For `s3`: the endpoint origin, such as `https://files.example.test`. |
| `PORTAL_REPORT_S3_BUCKET` | — | For `s3`: the bucket name. |
| `PORTAL_REPORT_S3_REGION` | `us-east-1` | For `s3`: the signing region. |
| `PORTAL_REPORT_S3_ACCESS_KEY_FILE` | — | For `s3`: a file holding the access key ID. Keys are only ever read from files. |
| `PORTAL_REPORT_S3_SECRET_KEY_FILE` | — | For `s3`: a file holding the secret access key. |
| `PORTAL_REPORT_DIR` | — | For `dir`: the folder for logs. |
| `PORTAL_REPORT_RETENTION_DAYS` | `30` | Days reports are kept, 1 to 3650. The privacy page shows it. |

## Reading reports

### Admin panel

`/admin/reports` (moderators and up) shows the last 14 days' counts per trigger and platform and the
newest 100 reports, filtered by player name, trigger, platform and whether they carry a player
comment, with the first 60 characters of each comment on one line. Each report's page shows its
metadata, the whole comment (line breaks kept), and its metrics, session and client JSON. Only
admins and owners can download a log; the download is the log decompressed (at most 64 MiB) as
`text/plain` attachment, through a form carrying the session's CSRF token, and goes to the audit log
as `download report log`.

### Operator commands

Run with `DATABASE_URL` set. `log` and `delete` also need the report store settings above
(`PORTAL_REPORT_STORE` and its variables).

| Command | Effect |
| --- | --- |
| `mag-portal admin reports list [--account NAME] [--trigger T] [--platform P] [--since RFC3339 \| --days N] [--has-comment] [--limit N]` | Reports, newest first (default 50): ID, time, player and account, trigger, platform, version, log count and size, and the first 60 characters of the player comment on one line (`-` for none). `--has-comment` lists only reports with one. |
| `mag-portal admin reports show <report_id>` | Metadata, the whole player comment (each line indented two spaces, or `comment none`) and the client, session and metrics JSON. |
| `mag-portal admin reports log <report_id> <name>` | One log, decompressed (at most 64 MiB), to stdout. Audit-logged as `read report log`. |
| `mag-portal admin reports stats [--days N]` | Counts per UTC day, trigger and platform (default 14 days). |
| `mag-portal admin reports delete <report_id>` | Deletes the report and its logs. Audit-logged as `delete report`. |

The panel and these commands leave out of a comment its control characters (other than line breaks
and tabs in the whole comment) and the invisible marks that reorder text, so it cannot drive a
terminal or disguise itself. The commands write the audit log as `operator (command line)`.
