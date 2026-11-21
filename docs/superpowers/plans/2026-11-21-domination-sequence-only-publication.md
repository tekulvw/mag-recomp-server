# Domination sequence-only publication implementation plan

> Execute the continuous gameplay-first TODO loop with the current Sol6.2 implementation agent. User directions supersede routine design/review approval prompts. No release or native run is included.

**Goal:** Let a qualified same-state objective entry publish its new playback sequence without aborting the Domination tick.
**Architecture:** Preserve authored MissionStart requests, accepted native entries, callback/effect ordering and atomic recording. Reuse bounded standalone ObjectiveSequenceDelta publication for qualified sequence rows without a changed control row; keep normal combined control/sequence publication for matching rows.
**Tech stack:** Rust1.99, existing replication codecs, synthetic transactions and private original-source profile replays.
**Baseline:** freshly fetched merged main5dc3c4fa666f8e453a7716f694a28507d8dd1ab0.

## Proved failure

The private Baku first tick at16ms produces90 changed controls and16 valid sequence-only rows. All16 correspond to original SecureHold constructor_state0 plus authored MissionStart transition_request0. The native entry is meaningful even when its control value is unchanged. Candidate::objective_packets rejects these at its unmatched-sequence guard. PR104 intentionally preserved same-state authored entry callbacks, so reverting that semantic fix or dropping these sequences is inappropriate. The visibility fix remains independent; source-bound recorded area checks pass with explicit isolated Completed-state preconditions.

## Constraints

Retain exact objective indices and qualified sequence ordinals/timestamps. Do not fabricate control-state transitions or weaken malformed/duplicate sequence rejection. Preserve existing packet size, recipient capacity, recording/rollback, replay and late-join behavior. Original retail files/evidence stay private. Driver/gunner authority is outside this fix, and APC driving follows after full-start continuity.

## Tasks

- [x] Add and observe a minimal MissionStart same-state entry RED producing a valid sequence with no control delta.
- [x] Exercise mixed changed/unchanged sequences across control-packet boundaries and standalone sequence budgets; retain strict malformed/duplicate/out-of-range validation.
- [x] Reuse the existing bounded standalone sequence codec while preserving matching combined publication and native effect order. Test record failure/retry without advancing playback/runtime/callbacks.
- [x] Run focused regressions, fmt/strictClippy, then bounded full suite/build/public boundary.
- [ ] Replay all six actual source-bound profiles through normal full-start objective updates without the isolated precondition; retain receipt of the prior original-source failure. Confirm area continuation as practical.
- [ ] Integrate focused public PR after green CI, exact checked-tree verification and private TODO/evidence checkpoint, preserving released pin and manual release PR. Continue into qualified APC driver control without another user prompt.
