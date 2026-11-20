# Domination runtime implementation plan

Implement the retail Domination server loop using externally generated map and
native evidence, reusable world transactions, and the shared result publisher.
All decisions about authored resources and native behavior require retail
verification. Debug symbols can locate functions for comparison.

## First integration

- [x] Reproduce original objective pool registration for all three maps, excluding
  trigger entities, and project all 32 squad lists with initial inactive state.
- [x] Generate six ordered faction profiles with 64-player bounded admission,
  four platoons per side, and explicit Basic fallback where APC insertion lacks
  a qualified runtime binding. Keep special insertion kinds unavailable.
- [x] Add source-bound controller extraction for the actually referenced primary,
  subprimary, delayed and tactical callbacks. Verify state observations, ordered
  requests and score-rate changes using synthetic tests.
- [x] Admit mode 2 initial objective catalogs with all 32 squad bindings and
  preserve the distinction between constructor state and MissionStart.
- [ ] Run source checks and actual private Engine/catalog consumers. Integrate
  independently working increments through focused source PRs.

## Connected gameplay

- [ ] Qualify MissionStart, current/desired state entry and callback timing,
  then stage ordered commands and publish objective and squad changes through
  record-before-install Engine transactions.
- [ ] Bind FSP and bunker lifecycle effects, interaction/repair/demolition,
  actor attribution and current collision/exit placement.
- [ ] Connect native score rates, terminal conditions, rewards/results and
  cohort release/requeue, with replay, late join and fresh-round reset checks.
- [ ] Exercise complete synthetic matches and all source-derived profiles.
  Native live-match acceptance follows under coordinated game ownership.

Use explicit gameplay configuration. Preserve proven current defaults and
manual release ownership. Native initial-load qualification alone enables no
interaction, score rule or post-start controller effect. Deferred fidelity
work follows the playable loop unless a concrete failure blocks it.
