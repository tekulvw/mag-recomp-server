# Configured independent live-world registry

The user approved the next server TODO: statically configure independent DME-world/Rust-worker pairs, route acknowledged admissions to their own worker, preserve per-world quorum/capacity and shared idempotent progression, and qualify stale-token/restart containment before dynamic provisioning. Continue checked source integration autonomously; no deployment, game run, account-store edits, release or released-pin update belongs to this task.

## Architecture

One Horizon registry owns a global saved-faction human queue and shared LocalCharacterStore. Between two and sixteen configured entries each own a LocalDmeHost, real distinct DME World ID, scoped GameplayBridge and one separate Rust process. The integrated DME listener and per-application manager remain shared; configured client IPv4/TCP endpoint plus actual world/token are returned only after that worker acknowledges reserve. Earlier result cohorts remain isolated within their owning host as PR105 implemented.

The registry fills compatible current worlds before starting another eligible cohort. Unassigned humans remain globally ordered; new formation selects the oldest feasible faction, randomizes opponent/orientation with existing faction logic, then binds the exact selected profile before forwarding an atomic batch to one host. Host pumps run after batch forwarding. A global account cannot queue/admit independently in two workers; terminal transfer first closes its old transport, while old result receipts remain owned by their original bridge. Cancellation and callbacks require exact current ticket/session ownership.

Registry mode requires retail application20231, worker profile support, independent result cohorts and worker_registry_v1. --worker-id opts Rust into a fresh process-incarnation identity. Replies carry worker_identity {worker_id,incarnation_id}; after discovery every request and subscription must present that exact identity before any engine/scheduler/ownership mutation. Horizon never adopts a replacement incarnation into an existing host. Stream heartbeats supply bounded liveness leases; failures stop new placement for that entry, preserving other workers. Match and reservation identities remain globally unique and exact receipts stay bound to the original worker/world/character. Shared filesystem persistence remains one authority in this process; automatic crash recovery of a lost Rust reward outbox is not claimed.

An explicit JSON registry supplies endpoint, IDs and per-worker minimum/maximum/team policy. Startup validates the entire shape and duplicate identities/endpoints before opening transport. Only drain flags can reload atomically at runtime; structural config changes require reconstruction. Draining stops new placement/admission while existing gameplay and settlements finish. Worker replacement is an operator reconstruction, not automatic provisioning/rebinding. Legacy single-host startup remains the default when no registry path is configured.

## Alternatives and limits

A list of extra Rust containers alone cannot route MAG clients. Multiple Engine instances in one Rust process would require isolating every runtime and scheduler; separate existing worker processes provide that isolation directly. General external-DME discovery, dynamic provisioning and multiple Horizon control-plane replicas are separate work. Configured pairs with a shared integrated DME listener satisfy this first source increment.

## Acceptance

Synthetic tests must drive two simultaneous worlds with independent faction cohorts, world/token/endpoint and worker fence, separate quorum/capacity and exact inbound/outbound routing. Cover stale callbacks/receipts/releases/tokens, wrong-worker or restarted-worker fencing, full/dead/drained placement, terminal requeue transfer and shared exactly-once progression. One worker failure must not stop another's ticks or cause old gameplay to be retried elsewhere. Preserve recorder-before-install semantics, immutable account identity, private-data boundary and manual-only releases. Source checks do not establish native live-match compatibility or throughput.
