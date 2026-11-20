# Leadership Progression Implementation Plan

> **For agentic workers:** Use parallel independent Sol6.2 source ownership and one focused final integration review. Standing autonomous/fast-loop instructions supersede routine proposal and per-task review gates.

**Goal:** Award source-qualified leadership points and persist correct rank advancement.
**Architecture:** Freeze native role in the existing terminal reward; Horizon owns authored arithmetic and atomic durable settlement; Rust consumes stable acknowledgements and publishes rank/match stat7.
**Tech Stack:** Rust, C# Horizon, existing atomic JSON store, neutral original-data extraction and private retail proofs.
**Spec:** docs/superpowers/specs/2026-11-20-leadership-progression-design.md

## Global Constraints
- Baseline public ea6ea29/treeb53f218e; private1399e68; released gitlinkf94705e unchanged. Preserve dirty native submodule and unrelated captures.
- Exact native current role1/2/4 at reward calculation; no invented weighting/history or fake account identities.
- Authored per-result/role awards and inclusive point+character-level thresholds; legacy inputs stay compatible and partial new tables reject.
- Leadership and XP share one receipt/finalization transaction, record-before-install and stable identity/deduplication.
- Historical unknown-point eligibility floor is an explicit migration policy, never fabricated points. New tracked records use exact source rules.
- Source integration only; manual releases. No original inputs publicly, XERJ, games/services/accounts/deployment/pin changes.

## Review Focus
- Role changes after reward creation cannot redirect/fatten the existing receipt; former leaders receive only the native rule.
- Zero-XP or zero-leadership-point outcomes still settle/publish correct metadata and can cross a character-level eligibility threshold.
- Duplicate/different-ID/changed-context/cross-character replay and uncertain writes cannot double-award or corrupt unrelated records.
- An older acknowledgement must not regress same-character state or update a replacement occupant; late joins must agree.
- Missing historical balances retain existing eligibility without creating points; source-normal new characters use rank0 below the minimum character level.

### Task1: Native contract and selected-resource qualification (native worker)
**Files:** private workspace server-leadership-progression-20261120/native only.
- [x] Verify retail current-role writes/read, result mapping, positive truncation, rank resolver/callers and selected original resource.
- [x] Retain reproducible private checker/receipt, distinguish remote-MLS unknowns and prototype data.

### Task2: Horizon catalog, persistence and bridge (store worker)
**Files:** horizon C# catalog/store/bridge/full-record/admission and focused tests only; no Rust edits.
**Interface:** optional terminal reward leadership context from spec; optional acknowledgement leadership record with optional leadership_points plus optional match_leadership_points.
- [x] Write failing synthetic catalog/store/bridge tests for full lifecycle, thresholds, minimum level, legacy/floor, rejection, rollback, replay and notification.
- [x] Implement source-driven catalog arithmetic, new-character initialization and tracked ordinary XP re-evaluation.
- [x] Extend immutable receipts and atomic finalization; preserve nominations/recency, optional legacy JSON and unrelated-account data.
- [x] Verify bounded offline SDK/container checks and report exact evidence/commit.

### Task3: Rust terminal capture and acknowledgement (root)
**Files:** progression.rs/new focused helper/tests, leadership_runtime.rs, lib.rs, match_flow.rs and affected synthetic fixtures.
**Interfaces:** RewardEvent.leadership:Option<LeadershipAwardContext>; LeadershipRecord.leadership_points:Option<u32>; ProgressionCommitted optional leadership/match_leadership_points.
- [x] Write connected failing tests for frozen actual native role, stale holder/replacement, zero/nonleader, both sides/results, one-shot terminal and record failure.
- [x] Capture context through the shared end-result path; keep legacy/prototype absent and preserve authenticated admission baseline.
- [x] Commit authoritative leadership snapshots/stat7/rank updates with existing acknowledgement recording, same-character reconnect and non-regression rules.
- [x] Test old/duplicate/invalid acknowledgements, out-of-round identity, current/late publication and rollback.

### Task4: Verification and integration (root)
- [x] Reproduce native proof and complete public docs/private TODO.
- [x] Run affected and final Rust/Horizon/Python checks, strict Clippy/fmt/build/boundary and focused review; fix concrete findings.
- [ ] Integrate focused public PR after required CI; verify exact merged tree, private main checkpoint and preserved released pin.

AutoHeal remains queued after this explicitly reprioritized gameplay work.
