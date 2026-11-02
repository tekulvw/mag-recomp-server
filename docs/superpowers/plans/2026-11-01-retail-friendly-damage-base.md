# Retail friendly damage base implementation plan

> **For agentic workers:** Use the existing subagent-driven source workflow with critical regressions and coordinator integration. The human has opted out of routine proposal/review gates.

**Goal:** Apply an external selected-match friendly-fire base before existing owned modifiers and armor in supported retail character damage paths.

**Architecture:** A strict optional disk rule binds application, map, native mode and exact spawn bytes. One current-world character relationship resolver supplies the incoming base. Armor retains native ordered additions; grenade queues retain their already-resolved amounts.

**Tech stack:** Rust/serde/SHA-256 and private Python projection with synthetic tests.

**Spec:** [Selected-match damage rules](../../character-damage-rules.md).

## Constraints

- Extracted values remain external; only independent synthetic examples enter public checks.
- Preserve accounts, audit-before-commit, ownership, generations, rewards and the existing 1000 HP policy.
- No game/service/deployment/release/pin changes. Falling and unsupported retail bunker paths remain separate.

## Tasks

- [x] Confirm actual incoming Bullet/Swipe/Explosive categories and owned modifier bindings against retained original evidence; keep response/object/source categories distinct.
- [x] Add failing strict rule/profile and explicit CLI tests; implement disk loading and exact spawn-source identity.
- [x] Add failing additive/pre-armor, typed hit and queued-blast regressions; wire per-recipient base without a second queue scaling.
- [x] Verify source relationship reuse preserves healing-credit behavior and source-world boundaries.
- [x] Run the affected/full synthetic Rust checks, strict Clippy/fmt and public-tree guard. Prepare one focused source PR and a stable-head handoff for the coordinator.
- [x] Independently verify the private projector/output receipt and prepare its private-only integration handoff.
