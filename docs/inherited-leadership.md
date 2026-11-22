# Retail inherited leadership publication

Configured retail leadership now publishes the committed inherited ability tier
in Character family3 field19. This is a single unsigned byte within the existing
22 interleaved Character field-presence bits. Values0–3 name inherited ability
tiers; they are separate from the saved progression leadership rank in field18.
The field remains server-owned: owner reports containing field19 remain rejected.

The original field defaults to0. Its native client consumer compares the network
byte with its cached inherited tier and changes the local CharacterModifiers
table by adding or removing the intervening authored tiers. The client reload
path reads the resulting general ReloadTime modifier (ordinal23) together with
its category modifier. Bounded original movement consumers read MovementSpeed
(ordinal12) from that same table. This change supplies the missing network input;
it adds no server movement simulation, reload deadline, or new modifier rules.
Full native movement arithmetic and live-match presentation are outside this
source qualification.

## Existing policy and current state

Publication is a read-only consumer of the existing committed PassiveState.
LeadershipCatalog, role elections, provider range, hold/grace, state and health
policy, and existing server modifier consumers keep their current behavior.
World, life generation and squad must match the cache scope. A stale cache reads
as0. Incapacitation does not acquire a separate publication immunity or clear
rule; it follows the existing policy and cache scope.

Only the existing optional retail leadership configuration enables this path.
Unconfigured and prototype paths emit no additional frames. A fresh zero tier
uses the native constructor default without a new delta. Once an observer has a
nonzero tier for a wire index, a downgrade or current-life/squad replacement
publishes the desired current tier, including0. Allocation replacement never
inherits authority from the previous reservation. A replacement with a current
nonzero tier needs one current row, rather than a duplicate clear/grant pair.

Each observer receives one ordered native ObjectUpdate containing all changed
Character rows. A full64-row frame is395 bytes, below the1397-byte packet bound.
These are ordinary reliable FIFO frames, so the shared drain's aggregate message
and byte budgets defer excess fanout across later drains. There is no direct
reply fanout proportional to both the roster and changed rows.

## Creation, recording and retry

An allocation-qualified introduction ledger is updated after the normal recorded
Character creation/snapshot enqueue. Peer deltas are eligible only after that
current allocation's snapshot has entered the observer's FIFO; own deltas follow
the recorded initial Character packet. Existing snapshots precede the level
frame for late joins. Character allocation creation survives respawn, while the
publication identity includes observer and subject world, round, reservation,
allocation, player, life and squad. Stale allocation entries are pruned at creation
and subsequent successful publications, bounding the ledger to the current
roster.

There are two recorded transactions. The existing gameplay transaction commits
its leadership cache only after its own record succeeds. The later publication
transaction records the queued packets before installing its publication ledger
or adding FIFO frames. Publication record refusal leaves the already committed
cache intact and leaves publication state and queues unchanged, ready to retry.
A full or backlogged observer FIFO defers that observer without advancing its
publication ledger or dropping reliable traffic. Normal shared-drain recording,
recipient sequence assignment and retained history still govern actual delivery.
The ledger means last queued, not network-acknowledged.

## Validation and limits

Independent MSB-first literals cover unsigned byte levels0,1,2,3. Connected
scheduler/publication tests cover self and known-peer grants, unchanged levels,
snapshot ordering, range/hold/grace clearing, life and allocation replacement,
squad/world fences, unchanged incapacitation policy, both record-refusal stages,
full-FIFO deferral, quiet zero defaults, bounded stale-slot cleanup, and client
write rejection. The source-qualified native contract is reproduced privately;
no original game code or assets are distributed here.
