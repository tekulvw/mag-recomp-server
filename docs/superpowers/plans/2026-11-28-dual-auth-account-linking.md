# Dual authentication implementation plan

> **For agentic workers:** Execute the bounded tasks below with test-first checks; independent portal and Sony-verifier tasks can run concurrently. Preserve source ownership, do not merge or deploy.

**Goal:** Accept portal and explicitly trusted Sony NP tickets through the same account/session path, with proof-based browser account linking.

**Architecture:** Separate ticket verifiers share bounded reassembly and canonical account admission. Portal PostgreSQL owns durable links and hashed, short-lived pairing proofs; portal-only mode stays unchanged.

**Tech Stack:** Existing .NET 10/Npgsql/BouncyCastle, Rust/Axum/SQLx/PostgreSQL.

**Spec:** `docs/superpowers/specs/2026-11-28-dual-auth-design.md`.

## Global constraints

- Source and synthetic tests only; no real credentials/tickets, games, deployments, merges or releases.
- Preserve `off|required` behavior. New `dual` mode requires explicit keys, services and database configuration.
- No name-based linking, unknown-key acceptance or signature-failure fallback.
- Preserve current canonical owner keys, portal ticket bytes, session leases and supported header checks.
- All public builds work without private inputs. Console acceptance remains unchecked until hardware evidence exists.

## Review focus

- A valid external name equal to another portal name must not grant its characters.
- Link redemption must prove both identities atomically and withstand concurrent redemption.
- Database outage, ban, deletion and unlink must deny new console admission.
- A console code or ticket must never enter diagnostic logs.
- Linked differing online IDs must pass identity consistency without changing storage ownership.

### Task 1: Sony NP parser and signature verifier

Files: new `MagPsnTickets.cs`, `PsnTicketChecks.cs` and public-source references.

Interface: `MagPsnTicketVerifier(IReadOnlyDictionary<string, byte[]> keys, IReadOnlyCollection<string> services)`; `Verify(byte[] bytes, long nowUnixSeconds, out MagPsnTicket ticket)` returns existing `MagPortalTicketReason`. `MagPsnTicket` exposes `Subject`, `OnlineId`, `ServiceId`, `ExpiresAt`, `ReplayNonce` (16-byte authenticated-ticket digest prefix). Key IDs are eight lowercase hexadecimal characters; keys are DER SPKI EC public keys. Subject is `256:<decimal unsigned account ID>`.

- [x] Write and observe failing synthetic signature/format/time/service/identity tests.
- [x] Implement strict public-format parsing, bounded lengths, signature verification and allowlists; generate test keys at runtime.
- [x] Run focused .NET checks and record public implementation references and remaining MAG-specific gates.

### Task 2: Portal schema and browser linking

Files: migration `0009_external_identities.sql`, `portal/src/console_links.rs`, web routes/templates and database/HTTP tests.

SQL contract: `external_identities(provider text CHECK provider='psn', subject text, account_id integer REFERENCES accounts ON DELETE CASCADE, online_id text, created_at timestamptz DEFAULT now())`, primary key `(provider,subject)`, unique `(account_id,provider)`. `console_link_proofs(code_hash bytea PRIMARY KEY CHECK length=32, provider text, subject text UNIQUE, online_id text, expires_at timestamptz, created_at timestamptz DEFAULT now())`. The C# service inserts proofs only after successful Sony verification; the portal hashes exactly the lowercase 32-hex-character code string after trimming outer whitespace.

- [x] Write failing database tests for code redemption, expiry/reuse, conflicts, concurrent redemption, bans and account ownership.
- [x] Implement authenticated/CSRF-protected list/redeem/unlink page at `/account/console`, linked from the existing account page.
- [x] Redeem under a transaction that locks the account, rechecks person/name/ban, deletes-and-returns one unexpired proof, and inserts the unique identity; do not consume proof on rolled-back conflicts.
- [x] Existing Google/device/ticket endpoints and account models remain unchanged. Test deletion cascades and existing portal flows.

### Task 3: Server resolver and integration

Files: new `MagExternalIdentities.cs`, existing `MagPortalTickets.cs`, MAPS/MAS sessions, character creation and Program configuration/logging.

Interfaces: a verified Sony identity resolves through `external_identities JOIN accounts`; a portal numeric ID resolves directly through `accounts` and verifies the signed name still matches. An unlinked Sony identity receives a hashed database proof and a SCERT system message only on its own connection. Root owns this integration and the schema contract above.

- [x] Write failing policy, linking-store and character/session tests before implementation.
- [x] Add strict `dual` configuration, database resolver, independent Sony replay cache and shared session claims.
- [x] Carry authenticated online ID through sessions, validate it before canonicalizing stored NP IDs, preserve record validation and downstream credentials.
- [x] Make auth payload redaction unconditional by message type; never log pairing codes.
- [x] Verify existing portal tests and synthetic dual MAPS/MAS/character/MLS behavior.

### Task 4: Verification, console checklist and PR

- [x] Run public-boundary, affected Python checks, .NET suite and portal Rust formatting/clippy/tests against throwaway PostgreSQL.
- [x] Review complete branch for auth/ownership defects; fix findings with focused regression tests.
- [x] Add operator configuration and explicit console acceptance checklist distinguishing synthetic evidence from hardware gates.
Publication handoff: commit the verified changes, push the feature branch and open a PR; do not merge or enable automatic merge.

## Review resolutions

Preserved signed portal service-account admission while retaining person-only Sony linking. Added real-database regressions for concurrent proof issuance and proof-row lock expiry. Preserved exact Sony milliseconds through admission and proof deadlines. Masked ticket/MLS credentials in wrong-profile, malformed-frame and debug diagnostics. Dual-mode plugin work runs in a bounded per-channel queue off the shared tick; immutable application binding and monotonic activity publication fence worker completion.

Offline acceptance: 186 complete Linux server checks (including required PostgreSQL checks), 133 portal tests, 469 Python tests, 39 focused chart tests, Rust format/Clippy, Helm lint and public-tree/whitespace checks passed. Console-only checklist entries remain unverified.
