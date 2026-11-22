# Helo and indexed Hospital redeploy

Owned human redeploy supports current Helo passengers and indexed Hospital
passengers and gunners, alongside the existing Ground routes. The native
KillSelf owner check and terminal state7 apply to each supported family. Legacy
Alpha and other unqualified mounts retain their separate support boundary.

## Helo occupancy

The terminal event validates the current carrier source, actual passenger range,
creation, faction and squad. The Character and fleet must have one exact
matching occupant, with no conflicting mount family. The event removes that
passenger from a staged fleet and clears the Character's seat on the same
recorded commit. Another passenger remains in place, and the freed slot is
immediately available without waiting for the next world tick.

This is terminal release, so it needs no disembark pose, exit cast or operational
hull-health condition. The source allocation and state14 qualify the associated
vehicle before release. Existing self-kill source0/index-1 and body-map predicates
remain with the shared death-notice planner.

## Hospital ownership

The terminal event validates the exact indexed Hospital and seat, unique current
occupancy, completed creation and any current turret grant. It stages the new
family13 owner and turret owner from the remaining actual occupants. The life
commit clears the seat and revokes inactive gunner equipment authority; the
mandatory ownership correction reaches current recipients through their reliable
queues. A new gunner receives a fresh grant version.

The ownership packet is separate from optional turret aim updates. Its preflight
reserves both the correction and any terminal health frame that must enter the
same reliable FIFO, including the redeploying owner's ordered-health case. A
full queue refuses before changing life, seat, grant or ownership.

## Transaction and acceptance

Fleet and ownership candidates enter the event plan only after the complete
redeploy is admitted. Refused replication-history budget and failed recording
leave the original mirrors and ownership intact. Synthetic native-event tests
cover immediate seat reuse, other occupants, state7, grant takeover, history and
queue limits, stale/conflicting mirrors and record failure.

Original-file evidence qualifies owned mounted KillSelf and exact native seat
release. These source checks establish no live controls, camera restoration or
presentation acceptance. Human live-match review and release cutting stay manual.
