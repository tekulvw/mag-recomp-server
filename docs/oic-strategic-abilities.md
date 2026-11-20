# OIC strategics

Source-backed Domination supports Company election, authenticated strategic
requests and the five strategic gameplay consumers. This extends the same
recorded Engine candidates, ability timers and reliable publication used by
squad and platoon commands. Native client presentation and live-match acceptance
remain separate from source verification.

## Authority and configuration

Project the role policy and active-ability rules from original retail inputs with
the maintained extraction tools. Schema-4 policy adds Company nomination and
vacancy election for Domination only. Candidates come from actual squad member
order and the saved eligible nomination; a held lower role can be promoted.
Ties retain the first candidate. Promotion, lower-role replacement, withdrawal
and departure reconcile together. Holding a role does not modify account
progression, and development role bindings cannot grant command authority.

Strategic requests carry only the ability code. The server derives the current
Company, team and leader from the authenticated connection. A dead character,
wrong leader, unavailable mode/side, active jamming or cooling timer rejects the
request without spending timers. Replacing the OIC does not reset round-owned
cooldowns. Complete schema-4 Domination configurations require projected
strategic tuning in both standalone workers and formation registries, regardless
of Engine builder order.

## Consumers

| Strategic | Gameplay behavior |
| --- | --- |
| Sensor Sweep | Scans one opposing squad per native update and refreshes five-second detection for all allocated members, without requiring movement samples. |
| Signals Jamming | Locks the enemy Company's new squad, platoon and strategic requests. Already-running effects continue. |
| Tactical Strike | Immediately clears friendly squad/platoon individual activation timestamps, including published ability data. The shared platoon gate and running effect schedules remain intact. |
| Blockade | Applies the authored enemy reinforcement interval modifier to future player and shared vehicle wave renewals. |
| Rapid Redeploy | Applies the authored friendly reinforcement interval modifier to future player and shared vehicle wave renewals. |

The authored mode/side inventory controls admission: Blockade belongs to the
attacker and Rapid Redeploy to the defender. All durations, cooldowns and spawn
modifiers come from the external projected rules. Tactical Strike is a command
cooldown refresh. It does not create a blast.

A strategic interval change preserves the current wave deadline, including an
overdue deadline awaiting its manager update. The next native renewal uses the
current modifier once from the observed server time. Initial player insertion
delay is unchanged. Bots whose physical spawn is obstructed retain their due
wave and retry after the obstruction clears. Ground and helicopter class waves
use their existing shared scheduling consumers; no per-vehicle timer mapping is
invented.

Sweep retains native squad scan order and performs its final scan on the strict
expiry update. Detection publication tracks allocation and life identity, so a
replacement life receives its own update. Jamming does not invent a satellite
jammer flag or suppress already-running Sweep/Sensor detection.

## Commit and publication

An accepted strategic, tactical refresh and planned player-wave period change
install only after durable event recording. World updates capture strategic
expiry, scan cursors, detections and wave state in the enclosing candidate.
Failed recording leaves live authority available for retry.

Company snapshots include the native owner, leader, active/available masks and
ability data. Existing recipients receive deltas and late joiners receive current
state. A full reliable FIFO defers publication; failed recording changes neither
the recipient baseline nor its queue. Synthetic connected tests cover held-role
admission, side availability, jamming, cooldown refresh, replacement lives,
late join, FIFO pressure and failed-record retry. These checks do not establish
native visuals, complete live gameplay or performance. Releases remain manual
after the maintainer's live-match review.
