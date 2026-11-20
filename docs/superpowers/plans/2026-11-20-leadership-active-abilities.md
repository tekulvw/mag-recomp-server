# Leadership active abilities implementation plan

> For agentic workers: use subagent-driven-development for the independent
> source/state, effects and role prerequisites; root owns the connected Engine
> integration. Track each task's tests and source evidence before integration.

**Goal:** Execute the first mode-enabled retail squad strike and sensor support
from authenticated in-game requests, with native state publication and timers.

**Architecture:** Keep immutable qualified setup separate from per-round runtime.
Use the current Engine roster and recorded world transactions for admission,
scheduling, damage and publication. Preserve the existing disabled foundation and
explicitly opt in through validated runtime configuration.

**Tech stack:** Rust, existing bit codec/collision/damage modules, Python standard
library extraction tools, offline original ELF/archive analysis and Ghidra.

**Spec:** `docs/superpowers/specs/2026-11-20-leadership-active-abilities.md`.

## Global constraints

- Fresh public main baseline: `11d0ef157d260fb3f4f8ee66433d213b29d3b390`.
- All native decisions and data must be qualified against the retail build.
- Preserve the released private source pin and all unrelated accounts/services.
- No XERJ, game launch, deployment or automatic release publication.
- Public code/tests use external inputs and synthetic data; native receipts stay private.
- Preserve source compatibility for existing default-off configurations.

## Review focus

Pin wrong actor/role/squad and stale formation rejection to normal event tests.
Pin idempotent request/selection versus a new activation to cooldown tests.
Pin failed-record rollback across timer, scheduler, damage and publication.
Pin failed native activation separately from locally rejected input.
Pin late-join creation/state order and end-of-round cleanup.

## Task 1: qualify native admission and publication

Owner: activation worker. Proposed public files `active_ability_state.rs` and
its synthetic tests; root owns changes to the existing foundation module.

- [ ] Reproduce the retail mode initializer and per-role/side command selection.
- [ ] Qualify role/target/asset/anti-air, initial readiness and cooldown semantics.
- [ ] Qualify net-ability data, owning field/group identities and publication order.
- [ ] Agree concrete state/codec interfaces with root before public edits.
- [ ] Write and run failing behavioral/bit-layout tests, then implement the codec
      and source-bound state model; run affected checks.

## Task 2: qualify and implement ClusterBomb and Sensor effects

Owner: effects worker. Proposed files `active_ability_effects.rs`,
`active_ability_sensor.rs`, their synthetic tests and a neutral projector.

- [ ] Read original properties and verify their consumers and timing units.
- [ ] Qualify random sampling, target/contact/surveillance rules and cleanup.
- [ ] Agree `Effect`/prepared-work interfaces and native update order with root.
- [ ] Write failing scheduler/target tests; implement bounded original behavior.
- [ ] Project required data from explicit original inputs and reproduce privately.

## Task 3: enable native squad authority in match profiles

Owner: leadership worker. Files `leadership_runtime.rs`, leadership-only guards
and resets in `match_profiles.rs`, and focused synthetic tests. Root owns CLI.

Interface: add `Engine::command_leadership_role(&Active) -> Option<CommandRole>`.
It returns a role only for the identity held by native election; it does not
make alive-state or cooldown decisions and does not grant a development role.

- [ ] Write failing tests for native builders in both installation orders,
      preserved legacy rejection, global squad indices and saved nominations.
- [ ] Admit only native policy compatibility for registry mode; expose the held
      role query and reuse it where appropriate without changing legacy passives.
- [ ] Clear held roles on committed new-world retirement/selection; preserve
      them for idempotent selection. Test failed record and replacement behavior.
- [ ] Run the affected leadership/profile suite and report the interface.

## Task 4: connect configuration and recorded Engine activation

Owner: root. Proposed files `active_ability_config.rs`,
`active_ability_engine.rs`; existing `events/abilities.rs`, `lib.rs`,
`resources.rs`, `main.rs` and non-leadership portions of `match_profiles.rs`.

- [ ] Add meaningful synthetic foundation tests; retain legacy authority-bit refusal.
- [ ] Lock the qualified worker interfaces and source-input schema in this plan.
- [ ] Add explicit optional setup to fixed/profile startup and catalog validation.
- [ ] Test normal request authorization, native availability, timer preservation
      and failed recording before replacing execution-pending handlers.
- [ ] Prepare and record activation before installing state/publication.
- [ ] Integrate bounded native effect updates at the qualified manager phase,
      reusing the authoritative world/damage transaction and normal peer queues.
- [ ] Test late join, cancellation/round cleanup and role/reconnect boundaries.

## Task 5: source qualification and integration

- [ ] Exercise qualified source profiles privately through normal Engine requests;
      preserve exact input, source and receipt hashes and stated limits.
- [ ] Run affected checks, then the complete public Rust/Python suites, strict
      Clippy/fmt and the public boundary check once the connected code is stable.
- [ ] Integrate a focused source PR after required CI passes; compare merged tree.
- [ ] Update the private TODO/evidence checkpoint without changing the released
      source pin. Keep native live acceptance and releases manual.
