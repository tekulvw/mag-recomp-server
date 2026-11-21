# Independent Match Cohorts Implementation Plan

> Root implements Rust lifecycle and integration; a parallel Sol6.2 agent owns Horizon transport/admission. Standing autonomous fast-loop directions replace routine design approval and per-task review gates.

**Goal:** Admit the next match independently while old terminal cohorts finish results safely.
**Architecture:** One live world plus at most eight results-only cohorts. Shared durable receipt outboxes remain scoped by match and exact authenticated identities. Capability-gated Horizon cohort transports preserve the same physical wire WorldId.
**Tech Stack:** Rust, C#/.NET10, synthetic tests.
**Spec:** ../specs/2026-11-21-independent-match-cohorts-design.md

## Constraints and review focus

Manual releases only; no game/deployment/accounts/private catalogs in public. Preserve recorder-before-install ordering, legacy-worker behavior, startup quorum and faction routing. Exercise late old ACK/release/event traffic after slot reuse, deferred desired-kit receipts, cohort-local results readiness, absent/partial capabilities and bounded retention.

### Task 1: Rust results-only lifecycle

Files: new gameplay/src/match_cohorts.rs and match_cohort_tests.rs; lib.rs/resources.rs, match_profiles.rs, multiplayer.rs, match_flow.rs and world_tick.rs.
- [x] Write and run failing connected terminal rollover tests.
- [x] Introduce `ResultsCohort` with actors, round, terminal, reward_match_id, participants, profile and drain cursor; bound retention at eight cohorts.
- [x] Reuse round publication planning for live and retired actors, but never execute gameplay for retired requests. Preserve replay and countdown/statistics ordering.
- [x] Integrate fresh profile selection/fixed-profile reserve rollover, retirement, idle expiry and world ticks. Preserve outboxes and reject retired identity reuse.
- [x] Run focused tests and record RED/GREEN evidence.

### Task 2: Durable settlement across cohorts

Files: gameplay/src/progression.rs and focused synthetic receipt tests.
Interface: `Engine.results_cohorts: BTreeMap<String, match_cohorts::ResultsCohort>` keyed by reward match identity; each cohort exposes pub(crate) actors and participants. Live active/peers retain existing layout.
- [x] Test old deferred ACK after new admission with same character and a reused slot occupied by someone else; include recorder failure/idempotent retry.
- [x] Apply match statistics only to the matching cohort; propagate durable account snapshots to the exact character in other cohorts without their match totals.
- [x] Run affected reward suites.

### Task 3: Horizon admission and transport isolation

Files: horizon/Server.Medius/Medius/LocalDmeHost*.cs, owned DME cohort-container membership helpers and Tests.MagPlugin.
- [x] Demonstrate current global result6 barrier with synthetic bridge/store/world fixtures.
- [x] Gate independent lifecycle on `independent_match_cohorts_v1`; preserve old worker behavior.
- [x] Isolate finished transport membership/allocator and ensure late close/notifications cannot touch a successor.
- [x] Admit successor faction cohorts while old terminal members/receipts remain; preserve active account and pending-live-release barriers.
- [x] Run affected synthetic Horizon checks.

### Task 4: Verification and integration

- [x] Run Rust tests/fmt/strict Clippy/locked build, Python suite and public boundary scan; fresh review the combined change once.
- [ ] Publish focused source PR, wait for checks and merge the checked tree.
- [ ] Checkpoint private TODO and evidence, retaining the released pin and leaving release PRs open.

## Review resolutions

Fresh Sol6.2 review identified atomic fixed admission, last-terminal transport
retention/tombstones/desired-kit receipts, subscribed result FIFO ordering,
original wire-profile decoding and duplicate ACK metadata, and rejected Horizon
owner capacity. All have focused regression coverage; root's three lifecycle
failures and the two reward-order/kit failures were observed before fixes. Six
older fixtures now exercise reset at fresh admission and fresh reservation IDs,
retaining their original round/character/quorum assertions. Public source and
private evidence integration follow the complete affected checks and CI.

Final local validation: 930 Rust tests/35 suites, 64 Horizon synthetic checks, 266 Python tests, strict Clippy/fmt/locked build and 1,089-file public boundary pass. CRLF-aware whitespace checking preserves the existing DME file line endings. No game/services/account/deployment/release/pin change.
