# Independent match-cohort results and requeue

The user authorized the next core TODO and autonomous checked source integration. Retain one live gameplay world while earlier terminal cohorts finish results independently. No live game, deployments, account changes, release operations or released-pin updates belong to this increment.

## Design

Use bounded results-only cohorts containing finished actors, their frozen round outcome, wire profile, exact reservation identities, replay history and match participation. A fresh formation selection archives a terminal current cohort before installing the successor. Fixed-profile admission can perform the equivalent rollover. No unfinished gameplay world can be replaced. Pending durable rewards and desired-kit receipts survive rollover; late acknowledgements update their original match totals and only the authenticated character's durable snapshot in any successor, never its fresh match counters.

Keep existing native end/countdown/statistics/PostGame order. Gate results on that cohort's reward receipts rather than unrelated pending receipts. Archived actors accept only keepalive/time/ping/reliable replay and release; gameplay requests cannot reach the live world. Expire silent results transports using the existing idle deadline. Retain at most eight nonempty/unsettled results cohorts; refuse a ninth without destroying evidence or actors.

Horizon enables rollover only with the worker capability `independent_match_cohorts_v1`. Separate old and new transport membership and allocator state under the same wire WorldId/subscription. Preserve exact object ownership for teardown, delivery, session lookup and slot reuse. Legacy workers keep their existing barriers. Preserve faction selection, per-side quorum, saved faction and account identity. Full hosting of simultaneous live worlds remains separate.

## Alternatives

Waiting for all old releases preserves today's unnecessary global barrier. Dropping old actors truncates result delivery and is rejected. General multi-live-world hosting is a larger independent TODO. Bounded results retention is the selected implementation because the old cohort needs delivery, not continued world simulation.

## Acceptance

A participant requeues into a fresh admitted cohort while another old participant remains connected or has a late release/reward acknowledgement. Old clients still receive correctly scoped results and exact replay. New actors start with fresh round state, character life and sequence space. Late old traffic cannot damage, move, release, credit match counters or notify the new cohort. Recording failure does not install a rollover or consume receipts. All public fixtures are synthetic; original game data remains external/private. Releases remain human-only after live-match review.
