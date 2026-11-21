# Ordinary firearm assist credit implementation plan

> **For agentic workers:** Execute the bounded accounting, CLI and projection tasks in parallel with explicit file ownership and one Cargo lease. The human authorized autonomous source integration and continuation.

**Goal:** Settle source-qualified human and bot firearm assist counters on the first lethal collapse, with no fabricated XP or account identity.

**Architecture:** An optional external Rules document enables a bounded five-entry contributor ledger for each current victim allocation/life. Existing admitted ordinary firearm hits record opposing contributors before HP changes, including fully absorbed accepted hits. Existing recorded collapse transactions settle current eligible contributors once and clear the victim ledger. Human stat11 uses normal statistics publication; bot stat11 uses the creation-fenced absolute publication added in PR125.

**Tech stack:** Existing Rust engine/serde/native codecs and Python standard-library projection tools. No new dependencies.

**Spec:** Qualified ordinary firearm/statistics contracts. Original source proofs and extracted data remain private.

## Global constraints

- Rules fields are schema_version1, application_id20231, assist_timeout_ms u32; zero is a same-instant lifetime, absence disables the increment. No hardcoded gameplay timeout.
- Five records refresh an existing actor, fill first empty, otherwise replace earliest expiry with first-slot tie preference. Expiry is inclusive.
- Skip the killer and self/friendly contributions. Resolve exact current actor allocations; an attacker respawn preserves eligibility, a replacement allocation receives no prior credit.
- Key victim state by world, round, allocation and current life; settle first collapse including incapacitation and clear before a revived actor's new contributions.
- Use wide whole-millisecond monotonic stamps (the full Duration range plus a u32 timeout fits u128) as the replacement clock policy; do not reproduce the original unsigned network-clock rollover defect.
- Preserve existing damage, kill/death counts, legitimate rewards, recorder refusal and publication budgets. No invented assist XP, bot account, final-results archive or new weapon-class claim.
- Keep extracted files/private evidence out of public source. Releases and live-match acceptance remain human-owned.

## Review focus

- A fully absorbed admitted opposing hit can contribute; no invented HP threshold.
- Exact expiry, zero lifetime, duplicate refresh, sixth actor and equal-expiry replacement follow the stated rules.
- Death, bleed-out, finish, revive and respawn cannot double-settle or carry prior victim-life credit.
- Departed/replaced slots cannot inherit another allocation's contribution, while the same contributor may respawn.
- Recorder refusal cannot advance ledgers, counters, life or sent baselines.

## Task1: Ledger and recorded integration — Nextscope

Files: new `gameplay/src/firearm_assists.rs`, existing firearm/events/statistics modules, bot statistics/publication mechanical seams and focused tests.

- [ ] Write a real admitted ordinary Hit regression and watch missing assist fail; add five-slot boundary/expiry/zero tests.
- [ ] Implement Rules::load/validate, Engine::with_firearm_assist_rules, bounded State and detached record/settlement plans.
- [ ] Integrate human-to-human/bot and bot scheduler admitted hit/collapse paths. Extend bot totals with assists/stat11 and preserve creation ordering.
- [ ] Verify refusal, absorbed hit, actor replacement/respawn, victim incap/finish/revive and second collapse.

Interfaces: `Rules {schema_version:u8,application_id:u32,assist_timeout_ms:u32}`, `Rules::load(impl AsRef<Path>)->io::Result<Self>`, `Rules::validate()->Result<(),&'static str>`, builder returns `Result<Self,&'static str>`. Root supplies `Engine.firearm_assists:State` default initialization and public module declaration.

## Task2: Explicit CLI and projection — root/Area

Files: root `gameplay/src/{main,lib,resources}.rs`, Area `tools/project-retail-firearm-assist-rules.py` and focused Python tests.

- [ ] Add CLI regression for optional/absolute/retail-bound valid zero+positive and invalid rules; observe RED before flag wiring.
- [ ] Load `--firearm-assist-rules` before opening logs, report enabled validation/startup state and apply the builder.
- [ ] Project the named MAGCharacterStats AssistTimeoutMs from neutral authored rules, finding its actual Lua-table entry rather than assuming an ordinal. Validate matching source literal/member identity, retain receipt separately and test malformed/ambiguous/missing inputs with synthetic values.
- [ ] Generate/check actual private rules with the maintained tool.

## Task3: External-input replay and source integration — Driver/root

- [ ] Replay human contributor/bot finisher and bot contributor/human finisher on exact retained source inputs, plus inclusive expiry/expired/refresh/replacement and current victim life controls.
- [ ] Freeze source/input hashes, run affected checks and full Rust/Python suites, fmt, strict Clippy, build and public boundary.
- [ ] Integrate focused public PR after all six CI checks and exact merged-tree verification; preserve release candidate and released private pin.
- [ ] Checkpoint private evidence and continue the next qualified core gameplay TODO.

## Bounded publication correction

The admitted seven-recipient/four-contributor regression reproduced 35 direct rows exceeding the 32-message reply budget. Assist-only human rows now mark a bounded recipient-local allocation/fence map instead of adding direct replies. The existing shared drain selects latest stat11-only values after the queued creation prefix, alternates them with replication and bot rows, and consumes markers only after recording. This retains current full human statistics behavior and avoids stale queued totals or new fanout failures. Scheduling cursor/preference now belong to the recipient rather than requiring an optional bot registry entry.
