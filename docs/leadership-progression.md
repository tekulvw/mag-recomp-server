# Leadership progression

Leadership points are settled with the existing terminal match reward. The
server captures the authenticated character's actually held native role when
that reward is created. Squad, platoon and OIC roles select separate authored
award rows for major/minor victory/defeat. Earlier roles held during the match
are not substituted for the current role, and no service-time weighting is
invented. A later role change or slot replacement cannot rewrite the receipt.
Leadership points do not add ordinary character XP.

## Authored configuration

Horizon reads leadership rules from the same external `MAG_PROGRESSION_CATALOG`
used for ordinary progression. Supply the selected application's original-derived
`LeadershipLevels`, all three `LeadershipPoints_*` award rows and root
`AdditionalLeadershipPoints` setting. The existing neutral progression extractor
retains these inputs. Populated catalogs and original resources stay external.
A partial leadership section rejects catalog loading. Horizon advertises complete
leadership settlement support in private admission metadata. Legacy catalogs and
admissions without that support retain XP-only receipts. Receipts without the
new context retain their existing XP behavior; a qualified
leadership receipt cannot finalize against a catalog lacking its rules.

Rank resolution follows source row order and stops at the first unmet points or
minimum-character-level requirement. Equality qualifies. New tracked character
records and ordinary XP updates for tracked records use the same resolver. The native signed
comparison of the stored 32-bit points word is retained. Supported cumulative
addition refuses unsigned overflow without a partial account update; this is a
local settlement policy, not a claim about the unavailable original account
server's overflow handling.

## Persistence and compatibility

Leadership points, rank, ordinary XP, finalization and the immutable response
share one account/character/round store transaction. Same-ID replay returns the
original snapshot. Changed context, another character or another final receipt
for the completed round cannot pay again. A failed gameplay acknowledgement
record leaves Rust's outbox, points, rank and publication unchanged for retry.
The established uncertain-store-write behavior remains unchanged.

Saved leadership records may carry `leadershipPoints`. Historical rows that never
tracked a balance start tracking from zero without a fabricated point grant.
Their preexisting rank is retained through a private `leadershipLevelFloor` until
the source-derived rank reaches it. This is an explicit compatibility policy.
Freshly created or newly materialized rows have no floor and follow the source
resolver. A missing row materializes only from the trusted admission baseline
when a qualified terminal receipt is settled. Existing saved rows take precedence.
Saved nomination and matches-since-command bytes are preserved; this increment
does not invent their independent advancement or mutation rules.

The retail lobby sends the player's volunteer preference as plugin message
`0x0243`: the type followed by one byte of squad/platoon/OIC bits (1/2/4), with
no header and no reply. The client sends it and then its join request without
waiting, so the MLS session commits the byte to the saved record's nominations
before it handles the join, and that admission carries it. The bits are stored
as sent; gameplay masks them by leadership level at admission and elects among
the nominated humans. Bits outside 1/2/4 are refused and change nothing. A row
with no leadership record gains one at the level its native record already
publishes (1), with no point balance. The MLS log line's status names
the outcome and the bits received.

## Publication and acceptance

The existing native full character record carries cumulative leadership points
and rank. Acknowledgements update the matching character's current Rust snapshot,
match statistic7 and rank publication, including leadership-only rewards with
zero ordinary XP. Same-character reconnects receive their own award; replacement
characters sharing a slot receive none. Old-round acknowledgements cannot alter
current round totals. A newer cumulative snapshot can still refresh the same
character after requeue, while older snapshots cannot regress tracked state.
Admission serializes its record read and reserve against reward settlement so a
reconnect cannot install a stale pre-commit snapshot.

Synthetic tests qualify calculation, persistence, bridge shape, replay, recorder
failure and current rank/stat publication. The private retail checker binds the
original executable and re-extracted resource table. Native rank-up presentation
and live-match acceptance remain pending. Source integration does not authorize
a deployment or release; the maintainer cuts releases after live review.
