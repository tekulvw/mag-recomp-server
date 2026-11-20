# Platoon Active Abilities Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement task-by-task. Steps use checkbox syntax for tracking.

**Goal:** Add verified native platoon authority and permitted platoon command effects to the existing source gameplay path.

**Architecture:** Extend held leadership roles and use typed native ability scopes in the current round-owned command runtime. Reuse recorded world candidates and shared combat receivers. External neutral projections carry all authored settings.

**Tech Stack:** Rust, serde, Python extraction, private retail PPC/Ghidra evidence.

**Spec:** docs/superpowers/specs/2026-11-20-platoon-active-abilities-design.md

## Global Constraints

- Fresh public main 8640dc3; isolated codex/platoon-active-abilities branch.
- Manual releases; preserve released pin and unrelated SDK/services/accounts.
- Original inputs and proofs stay private. No XERJ or native gameplay runs.
- Retail verifies decisions; never implement unproved semantics by assumption.

## Review Focus

- A promoted squad leader must lose squad command authority and trigger squad replacement.
- Slot reuse, withdrawal and transfer must not inherit a prior leader's rights or damage credit.
- Squad and platoon ability instances must remain distinct while sharing the native platoon gate.
- Late join and queue saturation must preserve leader/data/mask publication order.
- A failed query or record must not spend an effect tick, damage or cooldown twice.

### Task 1: Native authority and election

**Files:** leadership_runtime.rs; focused native election module/tests; neutral leadership policy projector.
**Interfaces:** command_leadership_role returns the held native role; platoon_leader_index returns current player or ffff. Existing reconciliation entrypoint repairs both scopes.
- [ ] Verify retail eligibility, weights, member scan, promotion/removal order and command dispatch.
- [ ] Write and run failing regressions for promotion, ties, role withdrawal, replacement and failed recording.
- [ ] Implement vacancy-only platoon-first election with current reservation identity and source-bound weights.
- [ ] Run focused election/policy tests and commit owned source.

### Task 2: Qualified platoon effect schedules

**Files:** active_ability_effects.rs and focused new effect modules/tests; project-retail-active-ability-effects.py and synthetic tests.
**Interfaces:** typed EffectRules and Work carry effect geometry/metadata to the existing transaction; private native receipts define exact rules.
- [ ] Qualify timing, target requirements, damage filters and schedules for each effect from retail.
- [ ] Pin exact typed interfaces in the task ledger after qualification.
- [ ] Add failing synthetic effect/projector tests, implement minimal qualified schedules, and verify focused tests.
- [ ] Commit owned source, retain private checker/receipt separately.

### Task 3: Connected scoped runtime

**Files:** active_ability_engine.rs; active_ability_state.rs; active_ability_config.rs; connected synthetic tests.
**Interfaces:** native scope selects authority, availability, unit cooldown, retained Unit, effect identity and replication object. Effect work enters existing character/vehicle/objective transactions.
- [ ] Write failing authenticated command tests for held PL scope, wrong scope, cooldown and distinct squad instances.
- [ ] Extend scheduling, target admission, damage routing and publication using verified Task 2 work types.
- [ ] Test damage, failed/intercepted execution, rollback, role replacement, late join and queue pressure.
- [ ] Require complete source-bound effect inputs; regenerate private profiles and verify normal default loading.

### Task 4: Verify and integrate

**Files:** public ability docs; private TODO and owned native/source evidence.
- [ ] Run affected complete Rust/Python suites, strict Clippy/fmt and public boundary checks.
- [ ] Review focused diff; fix any concrete gameplay issues with regressions.
- [ ] Create focused public source PR, wait for required checks, integrate when green.
- [ ] Verify checked/merged tree equality; checkpoint private evidence and TODO on private main.
- [ ] Report source acceptance separately from native live acceptance. No release or deployment.
