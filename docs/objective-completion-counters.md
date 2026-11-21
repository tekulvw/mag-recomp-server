# Objective completion counters

The gameplay callback records the supported zero-XP counters with its objective and character state. No new Horizon reward aliases, account schema, XP, fractional carry, FRAGO points or capability flags are introduced. Optional XP account/character metadata and XP opt-in are unnecessary for these counters. The authenticated reservation remains their gameplay identity.

## Structural completion

A qualified Structural Repairing → Repaired entry selects the authored subtype: 3/4/5/7/8/11/13/14 increment counters 43/45/47/49/51/53/55/57, respectively. Other subtypes have no subtype repair increment. The current allocated defender latch owns the callback; death or a new life of that retained character does not by itself revoke it. The most recent qualifying owner tool packet can update the latch even after queued work already covers maximum health. Such a packet commits only ownership, adding neither repair work nor stat35/XP. Record failure preserves the prior owner and allows retry.

Deactivating → Repaired is a rebuilt-structure defuse, and earns the existing ChargeDefused reward/stat62. It does not increment a subtype repair counter or primary objective counter. This corrects the earlier interpretation of the shared Repaired callback. Normal Activated entry clears the defender latch; scripted wire actors alone do not establish completion credit.

## Vehicle capture

An ordinary Acquisition hold completion carries an explicit witness, bound to its pending target version and actor. Human admission freezes the reservation, world, slot, round and authenticated account even without optional XP metadata. Settlement requires that original identity; a replacement reservation cannot inherit the hold. At actual Completed entry, a validated active type-9 linked FSP increments VehiclesCaptured58 after its FSP/blocker effects succeed and before its controller and assignment refresh. Another occupant, vehicle mounting, delivery, a scripted/forced target or a superseded request cannot inherit this witness. The ordinary capture witness is independent of XP classification.

## Tagged objective completion

Qualified Completed, Deactivated and Repaired callbacks inspect the credited actor’s side’s 16 current squads. A matching effective tag compares objective family and slot, without the unused middle identity bits. Primary assignments, proximity and XP FRAGO eligibility do not substitute for a tag. Each matching squad’s current allocated published leader receives one TaggedObjectivesCompleted41 increment. A current held leader’s life, nominations or simultaneous higher role are not rechecked. Native vacancies and stale reservation identities grant nothing. Other configured leadership modes retain their existing published first-member policy.

Acquisition capture and delivery sample current assignments before their own controller changes them. Delivery retains a matching completion witness even without an account XP recipient, so its tagged completion counter remains independent of EscortVehicleDelivered XP admission. Structural callbacks use the staged mode’s assignment snapshot after common/FSP effects and before their own script.

## Transactions and compatibility

Counters and absolute StatUpdate publications share the existing owned character-effect candidate. The outer gameplay record precedes installation, so record failure releases neither counters nor messages. A repeated quiet tick does not create another entry callback. A later distinct repair/capture can count again. Existing participation state preserves committed counters for a same-character rejoin in the same round; the normal round reset clears match counters.

Positive XP remains in its existing immutable Horizon receipt path. When an opted-in XP receipt cannot fit the existing outbox, the whole callback waits for capacity, preserving its objective, counter and XP together for retry. Independence from XP admission does not bypass that transaction backpressure. Destruction’s companion ChargesDetonated61 remains on the previously implemented once-only destruction ACK. The original stats-enabled session scope and unsupported generic script provenance are not broadened by this increment.

Verification uses synthetic source tests and private original-retail byte/resource checks. No live-match presentation or results acceptance is implied. Releases remain manual after human live-match review.
