# Atomic Acquisition Callback Plan

**Goal:** Implement ordered prepare/record/commit and native score-rate primitives.
**Spec:** `docs/superpowers/specs/2026-11-02-acquisition-atomic-callback-design.md`.

- [x] Write red tests for ordering, failures at every phase, recorder rollback,
  stale/world mismatch, duplicate replay and monotonic bounded replay.
- [x] Implement callback adapter with explicit isolated backend stages; no default
  effect provider and no partial handling of unsupported commands/trailing calls.
- [x] Write red score-rate tests; implement only original-bound input/deadline rules.
- [x] Add typed per-pair damage dispatch and finite native mine mutation tests.
- [x] Reproduce original score/volume proofs and verify private graph model consumers.
- [ ] Run affected suites/fmt/Clippy/public boundary; integrate after CI passes.
- [ ] Checkpoint private receipts/TODO with concrete backend qualification open.

Review focus: source-order effects, current versus target state, complete overlap
pairs, infallible post-record installation, exact cached output, revision/world
identity, unsigned clock wrap and no implicit live-source qualification.
