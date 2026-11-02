# Remembering an in-match kit selection

Native `0x0252` stores a signed-byte desired inventory index; the gameplay server
retains its low three bits. Equipping remains a respawn operation. Remembering a
preference does not swap the current inventory handle, ammunition or equipped
kit, and does not rewrite saved kit contents.

## Committed work and ownership

A successful event reply alone does not prove that a selection was accepted.
The server plans a typed `CommittedLoadoutSelection` only for an admitted actor
with an authoritative saved inventory row, persistent account/character binding,
a valid primary and eligible faction/item bindings. Invalid desired indices keep
their native desired-state semantics but produce no persistence work.

The selection becomes pending only after the event audit and complete event
commit succeed. Each actor keeps its newest persistable choice with a checked,
monotonically increasing revision. A newer choice replaces older pending work;
revision exhaustion does not wrap or alter the equipped kit.

Successful replies may carry at most32 entries in optional `loadout_selections`.
Each entry has exactly these fields:

| Field | Meaning |
| --- | --- |
| `selection_id` | Opaque identity of the committed preference |
| `reservation_id` | Actual source reservation |
| `world_id`, `player_id` | Current world and native actor index |
| `account_id`, `character_id` | Actual saved-record owner |
| `revision` | Nonzero unsigned64-bit actor selection revision |
| `inventory_index` | Valid owned saved slot |

Horizon validates all identities against its **current** reservation. Retained
reward owners are not authority to change a preference after leaving. Its
settlement helper locks the same gate used by release across ownership checks,
receipt state and the atomic store write. Older revisions and conflicting
receipts never overwrite a newer preference.

`loadout_account_id` in admission is separate from the optional XP binding, so
remembering a kit does not enable progression without its configured catalog.
When a configured loadout catalog supplies valid authored defaults for a legacy
record with no saved row array, admission projects those actual defaults and the
record's remembered index. It does not fabricate or materialize saved rows. The
previous absent-row behavior remains available without a configured catalog.

## Settlement, retries and cancellation

After a durable save, Horizon sends a typed bridge request with operation
`loadout_selection_settled`, the reservation/world/player, selection identity,
revision and status `persisted`. A permanent refusal uses `rejected`. Either
matching status consumes pending work. A stale or duplicate acknowledgement
cannot clear a newer selection.

An unanswered event may already have committed. Horizon recovers typed pending
work through Hello/world replies; it never retries the raw native packet.
Repeated delivery of a saved receipt repeats only its acknowledgement. Transient
store IO uses a bounded three-attempt policy with one-second backoff; permanent
store failures settle as rejected rather than creating an endless retry hold.

Release, expiry and round replacement explicitly cancel unsettled selections.
An accepted but unsettled preference is therefore not promised to survive an
immediate leave. Already saved preferences remain in the character record and
are available on the next admission. Requeue uses a fresh reservation identity,
so an older owner's delayed receipt cannot overwrite the new owner's choice.

The store validates ownership and the selected primary before writing. Its
preference save preserves unrelated accounts, kits, progression, skills,
leadership, reward history and previously accepted extension fields. Strict
leadership schema rejection remains strict.

## Editing a saved kit while admitted

A successful MLS Armory save is durable before Horizon schedules any gameplay
refresh. The notification contains the authenticated account and character,
rather than the client's proposed inventory bytes. Horizon projects the latest
canonical record through the configured catalog for the currently owned
reservation. Without an active reservation, the next admission already reads
the saved record.

The typed `refresh_loadouts` bridge operation carries the exact reservation,
world, player, persistent account and character identities, a nonzero revision,
and the projected saved rows. Gameplay validates those identities and the
configured item/faction bindings before recording and committing the refresh.
Only saved inventories and their update IDs change. The native desired index,
pending preference, already equipped kit, current inventory handle and observed
ammunition remain intact. A subsequent physical spawn equips from the updated
saved rows; a late joiner still sees the current life's retained kit.

Refresh retries repeat a bounded, revisioned snapshot, never the native Armory
save. A lost response may conceal a committed refresh, so an identical revision
and payload is idempotent. Older revisions and conflicting reuse of a revision
cannot rewind newer rows. A new successful save supersedes older pending work;
release cancels that reservation's work and requeue uses a fresh owner identity.
Failed storage writes never schedule refreshes. A refresh failure does not turn
a completed durable save into a failed save response.

## Component compatibility and verification

Bridge version2 remains in use. Hello advertises the explicit capabilities
`committed_loadout_selection_v1` and `active_loadout_refresh_v1`; Horizon requires
both before configuring or starting the world and before queue admission.
Startup fails clearly if either required capability is missing. Use matching
component builds; do not mix them while admitting players.

Synthetic Rust tests cover native low-bit semantics, item/owner validation,
commit failures, coalescing, revisions, acknowledgements, reply recovery and
lifecycle cancellation. Horizon checks cover strict decoding, compatibility,
real temporary-store saves, unknown-field preservation, stale/requeued owners,
bounded IO, uncertain acknowledgements and release/write ordering. These checks
do not establish native live-match acceptance. Live review and every release
remain human-controlled.
