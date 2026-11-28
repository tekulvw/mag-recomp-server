# Player-name changes

This feature is independently based on main. It does not require PR 435 or Sony authentication. Do not enable it on an existing deployment until every server accepting the portal's tickets has immutable account-owner lookup configured.

## What changes

A signed-in person may change the public player name on their account page. The existing 3–15 ASCII letter/digit/underscore/hyphen rule and case-insensitive uniqueness still apply. The confirmed cooldown is **24 hours between actual changes**, including case-only changes; submitting the identical spelling is a no-op. The server rechecks the current name, account eligibility, target reservation and cooldown inside the transaction.

The numeric account ID, linked devices, Google sign-in, permanent game-owner key, existing character names and progress remain unchanged. Social relationships continue using account IDs. Game-admin/report ownership uses the permanent key while public labels can show the new name. Historical report labels remain evidence of the name used at that time.

Former names remain reserved to their owner and can be reclaimed by that owner after the cooldown. Other people cannot acquire them through ordinary claims or service-account creation. Deletion retains protected reservations. Explicit administrative recovery of a detached deleted account remains possible and recovers its original game owner and history; a live account's historical names cannot be transferred.

## Deployment order

1. Upgrade the portal and apply `portal/migrations/0009_player_names.sql`. It backfills immutable `accounts.game_account_name` from existing names, assigns it on first claim and protects it from subsequent changes. Keep name changes disabled while upgrading game servers.
2. Upgrade **every** Medius ticket audience and enable `MAG_PORTAL_ACCOUNT_LOOKUP=required` alongside normal `MAG_PORTAL_TICKETS=required`. Set `MAG_PORTAL_ACCOUNT_DB_CONNECTION_FILE` to an absolute mounted secret-file path containing an Npgsql connection string for the portal database. Configure appropriate TLS verification for that connection. No Sony trust configuration is needed.
3. Verify that each server resolves an existing numeric portal account ID to its original character owner. The database role needs only read access to eligible account IDs and game-owner keys. Missing schema, invalid owners and unknown IDs fail closed.
4. Only then enable `PORTAL_PLAYER_NAME_CHANGES=1` on the portal. It is off by default. The operator is responsible for confirming that all configured audiences have completed step 2; the portal cannot discover an unupgraded server automatically.

Old game servers treat the MAGT v1 public-name field as a character-owner key. Enabling renaming before upgrading them could show an empty character account under the new name. The explicit feature gate and rollout sequence prevent that unsupported configuration; this change does not claim compatibility with an unupgraded server after renaming is enabled.

The MAGT v1 wire format and ticket JSON fields stay unchanged for current native clients. A legitimate pre-rename ticket still identifies the same numeric account and permanent owner until its ordinary expiry. Restart MAG to apply a new public name consistently across the launcher, game identity and freshly issued tickets.

## Helm configuration

The existing game-server chart supplies read-only owner-lookup configuration to main Medius and MAPS only:

```yaml
portalTickets:
  mode: required
  audience: play.example.test
  keys:
    "1": <portal public SPKI key in base64>
  accountLookup:
    mode: required
    existingSecret: portal-account-database
    secretKey: connectionString
```

The existing Secret must hold the connection string under the configured key. The chart mounts it read-only at `/portal-account-db/connection-string`. It does not expose the secret or lookup environment to gameplay workers or initializer containers. Default `accountLookup.mode: "off"` preserves existing rendering and database-independent ticket verification.

## Runtime behavior

Medius verifies the existing ticket signature/audience/time fields, then resolves its signed numeric account ID through an immutable-owner snapshot. The public name is presentation and authenticated NP identity; it never selects a different character owner. Current and pre-rename signed names for that ID share the same owner and session exclusion.

Database refresh runs in the background, outside the shared game/client tick. Snapshots refresh every second, expire after five seconds and are bounded to 100,000 rows. A failed refresh invalidates admission until recovery; unknown, unnamed, banned or deleted accounts cannot be resolved after refresh. Existing in-game credentials and match progress are unchanged. This cache is process-local and does not create a distributed session lease.

New character requests validate the NP identity against the signed public name, then retain canonical owner-bound storage. Existing character records are not rewritten. Public-name presentation flows through MAS credentials, MLS and game admission without changing the selected character ID or granting ownership through a display name.

## Verification and remaining acceptance

Offline tests cover migration/backfill and owner immutability, cooldown and case changes, same-name races, former-name protection/reclaim, administrative recovery, deletion, devices/social/report identity, authenticated CSRF controls, signature-to-owner binding, cache expiry/outage behavior and differing names through character creation, MAS/MLS, cards and game admission. All test accounts and data are synthetic.

- [ ] Human deployment review confirms all ticket audiences use owner lookup before enabling portal renaming.
- [ ] Live native-client review confirms restart, refreshed tickets, character selection, HUD/card names and reconnect behavior after a rename.
- [ ] Any future integration with PR 435 reconciles its separate migration 0009, dual-mode resolver and console character aliases after this main-based PR merges.

No merge, deployment, console run or release is authorized or performed by this source change.
