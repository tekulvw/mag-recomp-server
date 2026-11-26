# Bot spawn exits and drop traffic

Autonomous bots often leave a spawn through certified drop passages (one-way
ledge drops recorded in the navmesh artifact). This page describes how a spawn
wave is spread over those exits and how a passage is shared. All behaviour is
controlled by the bot population's `objective_policy`; fields that a deployed
population omits take the bundled defaults from
`gameplay/data/synthetic-bot-objective-policy.json`.

## Exit choice for a wave

Each bot's route search adds a per-passage penalty: the service time of every
bot already reserved on that passage, converted to walking metres. Bots are
routed one after another, so a long queue at the shortest exit makes a slightly
longer exit cheaper and the wave spreads over every usable exit.

Passages influence each other's penalty in two ways:

| Relation | Detected by | Effect |
|---|---|---|
| Exclusive | Swept bodies of the two launch pads, flights or landings can meet | Queues are pooled in full; admission never serves both at once |
| Crossing | Queued ground approaches cross | Each adds `drop_crossing_share` of the other's queue |

Exclusivity is computed once per navmesh from the certified trajectories. The
coarse protected box (whole flight plus a square entry buffer) is no longer
used for this: parallel drops off one ledge whose bodies never touch are
independent exits. Set `drop_crossing_share` to `1.0` to restore the former
full pooling of crossing approaches.

### Exit first, then the goal

While a bot is still in its spawn (the start component of a route that needs
a drop), the first drop is chosen by how soon the bot is out: the walk to the
launch, the passage's queue penalty and the flight. The remaining distance to
the goal counts only with weight `spawn_exit_goal_weight` (default `0`, the
fastest exit; `1` restores choosing by the whole route). Later drops and
ground routes always use the whole route. Reconsideration of a queued bot
compares candidate routes the same way, so a bot is not pulled back to a
congested exit just because its landing is nearer the objective.

## Convoys on one passage

A passage used to admit one bot at a time from approach until it had landed
and cleared. Now, once the owner has left the launch pad in certified flight,
it becomes a departing convoy member (it keeps its permit) and the next ticket
is granted and may walk up behind it. The follower's permit to enter the
passage is withheld until every departing member has landed and moved at least
`drop_convoy_landing_gap_m` from the landing, so two bodies never share the
landing. `drop_convoy_limit` bounds the departing members per passage; `0`
restores single occupancy.

A follower that has already entered is not recalled if a departing member then
stops on the landing; the existing aborted-drop recovery applies in that case.

A departing member never takes a new ticket for the passage it holds. Once it
has walked out of the passage its live status no longer names the drop and the
registry falls back to its queue-time snapshot; queueing it again would freeze
it beside the landing, keep the landing occupied and stall every follower.

## Queue staging and planning budget

Queued bots wait on staging spots around the launch. Candidates are scanned
with cheap checks before any route query: spots in front of the ledge, spots
without ground within `ground_search_m`, a different navmesh component,
protected boxes, approach lanes and occupied spots are skipped. Up to
`drop_holding_scan` candidates are checked per attempt. A spawn floor below the
launch level is a valid staging area; only the navmesh component decides
whether a spot belongs to the launch side.

Among valid spots the scan prefers one the bot can reach without walking
through another body; in a crowd where every path crosses someone it falls
back to the first otherwise valid spot. A bot walking to its spot that makes
less than 0.5 m of progress for `drop_holding_progress_stall_ms` (default
3000) is restaged without waiting for the full `drop_holding_stall_ms`. Both
rules prevent two waiters that must swap sides in a one-body corridor from
blocking each other until the full stall, repeatedly.

On a narrow ledge or a layered platform the whole ring of
`drop_holding_candidate_limit` spots can fall off the launch's floor. Once a
ticket has spent a full ring without a spot, the bot holds where the queue
froze it (`bot_drop_holding` with `action: "stage_in_place"`), if it is
grounded on the launch's navmesh component, behind the ledge, and clear of
protected boxes, approach lanes and other waiters' spots. It stands on its own
route to the launch, so no route query is needed; waiters behind it form a
single file along that route.

Navmesh goal projection visits every polygon in its goal box. Layered floors
can put more than 512 polygons in a 5 m box; a fixed buffer used to fail such
goals (`navmesh_goal_candidate_limit`) or silently lose the nearest polygon.
Ground recovery back onto the navmesh searches only the cells a route of its
`maximum_m` can reach, so an unreachable target no longer costs a full 64-cell
grid search.

### Crowded passages

In a large world a passage can draw a crowd faster than it serves it. Each
bot is queued and frozen where it stands once it is within
`drop_queue_distance_m` of the launch, which is usually on the approach
itself. Once the staging ring is full, later waiters find no spot that is not
in a protected box, an approach lane or another waiter's place. A waiter
without a spot is never ready, and it cannot be asked to yield, because only
waiters on their spots yield. Ready waiters behind it then stay blocked, and
the passage sits idle with a full queue.
Domination sims at 256 characters showed queues of 15-23 waiters on one passage, which
admitted about one bot a minute.

When a passage has waiters, no owner, nothing in flight and no staged waiter
for `drop_crowd_grant_ms` (default 1500; `0` disables it), it grants the waiter
nearest its launch along its route whose approach is clear. That waiter
must be grounded, frozen on its route (not aborted), outside the passage's
protected box, and must hold an approach that conflicts with no other
passage's active lane. In a single file along the route, that is the waiter at
the front. It resumes its frozen route through the passage. Staged waiters
keep precedence: the fallback runs only when no staged waiter can be
granted. The grant receipt carries `crowd: true`.

When at least `wave_backlog` grounded bots are waiting for a first route or a
staging spot, the planner uses `wave_route_queries_per_tick`,
`wave_candidate_checks_per_tick` and `wave_planning_slice_us` (each only ever
raises the ordinary limit) so a spawn wave is routed together.

## Role destinations around raised objectives

A role spot (`attack_roles`, `defend_roles`, armed-objective duties) is an
offset from the objective at the objective's own height. Around a tower or a
rooftop that height can be in the air, inside a structure, or on a ledge the
eroded navmesh does not reach. When the ground search (`ground_search_m`)
finds no support there, or the supported spot is more than 0.3 m from the
navmesh (the route goal tolerance), the nearest navmesh floor within 3 m
horizontally and 16 m vertically stands in for it, provided the body is
supported there too; up to three floors are tried. A spot already supported on
the navmesh is unchanged. Operator approaches and recovery steps keep their own
search.

## Receipts

- `bot_drop_admission` with `action: "convoy_departing"` marks an owner moving
  to the departing set; `released` receipts carry `convoy_member`.
- `bot_planning_slice` carries `wave` and `backlog`.
- `bot_drop_delay_sample` service is the interval since the previous clearance
  when services overlapped, i.e. the passage's measured headway.

These are synthetic-tested mechanics; they make no claim about live-match
performance until reviewed in a real match.
