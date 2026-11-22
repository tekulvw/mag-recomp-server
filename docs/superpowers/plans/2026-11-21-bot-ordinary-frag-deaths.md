# Registry bot deaths from qualified ordinary frags implementation plan

> **For agentic workers:** Use the existing parallel owners and executing-plans workflow. Standing authorization permits focused implementation and integration without another proposal gate.

**Goal:** Increment the existing allocation-owned bot Deaths counter once when source-qualified ordinary frag damage causes its first positive-health-to-zero collapse.

**Architecture:** Carry private typed damage provenance from the accepted frag producer through the existing owner damage queue. At drain, revalidate world, round, life and bot allocation before damage, then prepare the existing victim-only K/D plan on first lethal collapse. Reuse event recording, health commit, counter install and creation-fenced absolute stat12 publication.

**Tech stack:** Existing Rust engine, ordinary frag/body pipeline and synthetic tests; private original-retail byte checks and direct Acquisition offline replay.

**Spec:** Existing `docs/retail-frag-character-blast.md` and the scope below. Private qualification reproduces the retail callbacks, shared first-death dispatch and stat12 increment before attacker processing; it does not distribute original program bytes.

## Global constraints

- Only the qualified ordinary-frag producer creates its private provenance. No client/wire flag or generic explosive/falling inference authorizes this counter.
- Preserve falling calculations and legacy/development/rocket/other damage behavior.
- Capture and revalidate world, round, target life and exact registry allocation before applying qualified queued damage.
- Victim eligibility is independent of attacker account/team. Preserve existing thrower kills/XP, assists, notices and final results; add none for bots.
- Preserve admitted postterminal damage grace while leaving frozen round counters unchanged.
- Record before health/counter installation. Stale, nonlethal, duplicate or refused damage cannot add a death.
- Same-allocation respawn retains totals; replacement allocation starts fresh. Use existing snapshot/creation/publication fences and packet budget.
- Runtime game inputs remain external and private. No game launch, service/deployment/account change, released pin update or release publication.

## Review focus

- An old queued dose must not hit a new bot allocation whose slot and life number happen to match.
- Shared queued damage includes unqualified sources; explosive kind and inactive slots are insufficient provenance.
- Friendly or accountless throwers must not suppress an otherwise valid victim death or acquire new rewards.
- Late admitted damage after terminal must not fail solely because the existing K/D plan rejects terminal mutation.
- Counter rows must retain the current allocation's creation fence and be installed only after recorded damage.

### Task 1: Private typed provenance and queued victim accounting

**Owner/files:** Next: `lib.rs`, `events/update_actions.rs`, any necessary narrow existing EventPlan seam and mechanical source-field migrations outside Area's producer file; existing/new consumer regression tests.

**Interfaces:** Private `PendingDamageSource::{Falling,Other,OrdinaryRetailFrag { world:u32,round:u64,bot_allocation_generation:Option<u32> }}` replacing the ambiguous falling boolean. Preserve falling behavior via enum match. Coordinate the exact type/field name with Area before compilation.

- [ ] Add the type and mechanically classify existing producers/tests, leaving `character_frag_damage.rs` to Area. Only its qualified branch may create the ordinary-frag case in production.
- [ ] Write a meaningful failing qualified bot terminal test against the current zero-death behavior, then stage victim-only `bot_firearm_kd::Plan::prepare(engine,world,None,Some(life))` on first HP>0→0.
- [ ] Revalidate provenance before damage and retain existing event record→health commit→counter install/publication. Skip new counter planning during terminal grace without changing existing permitted damage.
- [ ] Verify hostile and friendly/accountless victim totals, stale world/round/life/allocation rejection, nonlethal/duplicate/refusal, unchanged human awards, natural respawn/reuse and ordered stat12. Do not broaden legacy or development queues.

### Task 2: Qualified producer marking

**Owner/files:** Area: `character_frag_damage.rs` and its focused synthetic tests, consuming Task1's private enum.

- [ ] Add a failing producer provenance assertion through the accepted ordinary-frag damage path, then mark doses with current world/round and captured registry allocation for bot targets. Human reservation fencing remains unchanged.
- [ ] Verify the captured bot life/allocation is current before marking; retain owner terminal, overlap, cover, armor and loadout admission. No new blast geometry or damage scaling.
- [ ] Test human versus registered-bot provenance where represented and preserve stale/overflow/refusal regressions. Coordinate compiler readiness with Next under one Cargo lease.

### Task 3: Original-map queue and publication acceptance

**Owner/files:** Driver: private direct Acquisition replay and input/qualification manifests. Root: docs, full checks, source PR and private TODO checkpoint.

- [ ] Reproduce the retained ordinary-frag death qualification against the original retail ELF. Keep private checker/receipt and original inputs external.
- [ ] Use current direct Acquisition Structural/dynamic frag setup with a normal registry population. Run an admitted slot3 launch and due terminal through current bot-body overlap and ordinary update queue drain; no direct counter mutation.
- [ ] Verify one bot death/stat12, unchanged existing human eligibility, ordered creation-fenced publication, stale/refused negatives and no new bot reward or notice claims. Disclose any low-health/initial-position fixture setup.
- [ ] Run critical affected tests, full Rust tests, Python checks if affected, fmt/strict Clippy/build and public boundary. Integrate after six CI checks, compare checked/merged tree and retain private evidence. Continue TODOs; releases remain manual.
