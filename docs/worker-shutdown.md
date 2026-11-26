# Worker shutdown and drain

A served gameplay worker (`mag-gameplay --bind ...`) handles `SIGTERM` and
`SIGINT`. The container runs the binary as PID 1. The kernel ignores a signal
sent to PID 1 when no handler is installed, so before this change a
Kubernetes rollout's `SIGTERM` did nothing. The worker kept running for the
whole termination grace period, 30 seconds by default, until the kubelet's
`SIGKILL`.

## What the worker does

On the first signal the worker:

1. **Stops admitting.** `reserve`, `reserve_party` and `select_match_profile`
   are refused with `worker_draining`. Events, releases, result delivery and
   reward acknowledgements continue.
2. **Tells Medius.** When the worker's hello lists `worker_drain_v1`, Medius
   asks for `drain_notices` on `subscribe_world`. The subscription
   acknowledgement and every later frame then carry `worker_draining: true`.
   Medius logs `Gameplay worker draining: worker=... world=...` once and closes
   that world to placement at once. Other worlds keep admitting.
3. **Lets the match finish.** It waits up to `--drain-seconds` (default 0) for
   every human to leave. Humans leave when the match ends and its results are
   delivered, or when Medius releases them.
4. **Flushes receipts.** It then waits up to `--receipt-flush-seconds`
   (default 5) for Medius to acknowledge the rewards and loadout selections it
   has already reported. Medius settles them from world frames as usual.
5. **Exits with status 0.** It closes its journal first, so queued records reach
   disk.

The worker prints `{"event":"worker_draining",...}` when the drain starts and
`{"event":"worker_exiting","reason":"drained"|"drain_deadline",...}` with the
humans and unsettled receipts left. A second signal exits at once with status
143.

Players still in the match when the worker exits lose its world stream. In the
configured registry Medius then releases them at once, and their DME
connections close (`Gameplay world stream interrupted: worker=... failure=IOException`).
A legacy single-world Medius waits up to 20 seconds for a resume first. The
retail client then returns to the menu, as on any world-stream gap. A worker
that stops sending frames without closing the stream is released the same way
after 12 seconds. Medius never keeps a session on a world it can no longer hear.

Rewards that the worker reported but Medius had not confirmed when the process
exited are lost, as on a crash. Restart adoption logs them (`Gameplay receipt
lost with worker incarnation`); see
[configured world registry](configured-world-registry.md#worker-restart-adoption).

## Operator flow: drain a world, then restart it

To restart a worker without cutting a match short:

1. **Stop placement.** Set the worker's registry row to `draining: true`
   (`gameplayWorkers[].draining` in Helm). Medius reloads drain flags each
   second without restarting. No new match or late join goes to that world;
   the running match continues.
2. **Let the match end.** Wait until the world holds no players. Each departure
   is logged as `Gameplay reservation released: worker=<id> world=<n> ...`
   (and `admission accepted` lines count arrivals). After the match's results,
   its players requeue or leave.
3. **Restart the worker.** For example
   `kubectl rollout restart deployment/<release>-mag-recomp-server-gameplay-<name>`,
   or apply the upgrade. The worker exits as soon as it receives `SIGTERM`,
   because it is already empty. Medius adopts the new incarnation for that world
   only (see [restart adoption](configured-world-registry.md#worker-restart-adoption)).
4. **Reopen.** Set `draining: false` again.

Without step 1, run the worker with `--drain-seconds` set to a match length,
and give its Pod a termination grace period longer than the drain and the
receipt flush. The `SIGTERM` itself then stops placement and lets the match
finish, at the cost of holding up the rollout.

A Helm upgrade that changes a worker's Deployment also restarts Medius, which
disconnects every world (see the chart README). A worker that keeps running
through a Medius restart is reconnected cleanly
([`control_plane_rebind_v1`](retail-runtime.md#a-worker-that-outlives-medius-control_plane_rebind_v1)).
