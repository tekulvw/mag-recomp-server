# Portal player-name changes

Implement this feature independently from current public main. It must not depend on PR 435. Keep a separate PR open and unmerged; the human will resolve overlapping changes in PR 435 after this PR merges.

## Behavior

The public player name may change without changing account ID, devices, external identities, character-owner key, existing character names or progress. The confirmed cooldown is 24 hours between actual name changes (including case changes); an identical spelling is a no-op. Names retain the existing 3–15 ASCII character rules and case-insensitive uniqueness. A former name remains reserved to its owner and may be reclaimed by that owner after the cooldown; another person cannot acquire it through ordinary claims. Conflicting or simultaneous renames are atomic and cannot transfer characters. Deletion continues to reserve names. Authorized administrative recovery must remain explicit, not an automatic name-based ownership shortcut.

## Compatibility and rollout

Keep MAGT v1 and existing native client compatibility. Add an immutable lowercase `accounts.game_account_name`, backfilled from existing public names and assigned on first claim, plus rename history and timestamp in portal migration 0009. The portal issues the new public name in the existing signed ticket and JSON fields. New Medius configuration `MAG_PORTAL_ACCOUNT_LOOKUP=required` and `MAG_PORTAL_ACCOUNT_DB_CONNECTION_FILE` resolve signed numeric account IDs to immutable game owners. This optional lookup is independent of Sony authentication and keeps its database work off the shared game tick. Legacy required mode is unchanged when lookup is unset.

Renaming is disabled by default (`PORTAL_PLAYER_NAME_CHANGES=1` enables it). Operators must first upgrade every configured ticket-audience server and enable immutable-owner lookup; old v1-only servers otherwise interpret the new public name as a different owner. This is an explicit coordinated rollout requirement, not automatic backward compatibility with an unupgraded server. No deployment is part of the task.

Existing signed tickets remain scoped to the same numeric identity and immutable owner until normal expiry. A renamed account should restart MAG to apply its new public name consistently. A bounded asynchronously refreshed owner cache is permitted: no synchronous lookup may block world/client ticks, unknown or stale mappings fail closed, and current bans/deletion are reflected after refresh. Preserve portal service-account admission.

Carry the authenticated public name through MAS credentials and MLS/game admission so the public name can be shown without rewriting existing character records. All writes, progress, selection and ownership continue using the immutable owner and selected character ID. Make owner-to-portal joins in admin/reports use immutable keys; preserve historical report labels and self-report restrictions.

## Verification

Test DB backfill/immutability, new claims, user/session/CSRF controls, cooldown boundaries and case-only changes, concurrent target conflicts, old-name reservation/reclaim, deletion/admin/service paths, stale signed tickets, both ticket modes, unknown/stale/cache outage behavior, differing public/owner names through character creation, MAS/MLS and game admission. Keep all data synthetic. Run affected Rust/.NET/Python checks and CI; leave both PRs open and unmerged.
