# Portal player-name change implementation plan

> Execute with test-first checks. Portal and server work are independent source areas sharing the schema contract below. Do not merge, deploy or release.

**Goal:** Let a signed-in person change their public player name every 24 hours without changing character ownership.

**Architecture:** Immutable portal game-owner mapping, transactional public-name/history changes, unchanged v1 tickets, optional off-tick Medius account-ID resolution.

**Tech stack:** Existing Rust/Axum/SQLx/PostgreSQL and .NET/Npgsql.

**Spec:** `docs/superpowers/specs/2026-11-28-portal-player-name-changes.md`.

## Global constraints

- Independent main-based PR, no PR 435 dependency; no merges/releases/deployments.
- Rename off by default; operators explicitly enable it only after all audiences use immutable-owner lookup.
- Keep current ticket format and native clients; preserve account IDs, devices, owner keys, characters and progress.
- User-confirmed 24-hour cooldown, case-insensitive uniqueness, owned former-name reservation.
- Synthetic tests only, preserve unrelated sources/accounts/services.

## Review focus

- Public-name changes must never select a different character owner.
- Concurrent rename, claim, delete and recovery must not release another live account's names.
- Cache/database outages must not block active games or invent an owner.
- Old/current legitimate tickets remain tied to the same stable account ID.
- Administrative/report joins must distinguish public display names from immutable owner keys.

### Task 1: Portal schema, rename transaction and UI

Own `portal/` only. Add migration `0009_player_names.sql`: immutable nullable `accounts.game_account_name` (lowercase, unique when named), `accounts.name_changed_at`, and owned historical-name reservations. Backfill existing named accounts; first claim assigns the owner once. Account deletion preserves required tombstones. The server cache reads `SELECT id, game_account_name FROM accounts WHERE player_name IS NOT NULL AND banned_at IS NULL`; preserve person and service accounts.

- [ ] Write and observe failing DB/HTTP rename and ownership tests.
- [ ] Implement authenticated CSRF rename controls, feature gate and 24-hour cooldown; keep device/ticket/OIDC flow unchanged.
- [ ] Protect former names across ordinary claim, deletion, service creation and explicit administrative assignment/recovery.
- [ ] Use immutable owners for game-admin and report joins while displaying current public names and retaining historical evidence.
- [ ] Run complete portal tests, format and Clippy; document rollout and exact behavior.

### Task 2: Independent Medius owner lookup and public-name propagation

Own `horizon/`. Add `MAG_PORTAL_ACCOUNT_LOOKUP=required` and `MAG_PORTAL_ACCOUNT_DB_CONNECTION_FILE`, independent from Sony configuration. Signed ticket account ID resolves to immutable owner using a bounded background-refreshed PostgreSQL snapshot; no DB I/O on service/world ticks. Unknown/stale mappings reject admission. Existing required/off behavior remains unchanged by default.

- [ ] Write and observe failing policy/cache and different-name ownership tests.
- [ ] Implement bounded owner cache and lifecycle/startup checks, preserving service accounts.
- [ ] Carry signed public name separately through sessions/credentials/MLS/admission; validate incoming NP IDs against authenticated identity while preserving stored owner records.
- [ ] Test backfill/database lookup, stale/outage handling, current and pre-rename tickets, and preserved character ownership/admission.
- [ ] Run complete Linux server checks with throwaway PostgreSQL.

### Task 3: Review, publication and CI

- [ ] Review both source areas for ownership and race defects, fix with focused tests.
- [ ] Run source-boundary and affected Python/Helm checks.
- [ ] Commit/push the independent branch, open the separate main-targeted PR and watch CI to completion.

PR 435 separately receives the console-name collision allocator and owner-scoped retry/login aliases; do not copy those changes into this branch.
