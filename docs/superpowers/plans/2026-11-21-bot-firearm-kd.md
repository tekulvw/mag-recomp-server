# Bot firearm kill and death accounting implementation plan

> **For agentic workers:** Use the existing parallel implementers for accounting and publication, with focused RED/GREEN checks and one combined verification. The human has authorized autonomous source integration and no repeated approval gates.

**Goal:** Count authoritative ordinary-firearm bot kills and deaths once and publish current absolute totals after each recipient's bot creation.

**Architecture:** Bot allocation totals survive respawn, while terminal admission still uses current world and life. Stage accounting in the existing recorded firearm transactions. A recipient-local creation fence admits unsequenced stat rows in the bounded fair shared drain; no replication sequence or replay-history entry belongs to a stat row.

**Tech stack:** Existing Rust engine, native application codecs, synthetic tests. No dependencies or runtime configuration changes.

**Spec:** Existing authoritative firearm/statistics contracts and the scope below. Private source qualification remains outside the public tree.

## Global constraints

- Preserve legitimate human statistics and rewards; create no account or XP identity for bots.
- Only ordinary admitted authoritative firearm terminals count here. Environment, other weapon classes, assists and final bot result archives remain separate.
- Count the first lethal collapse once, including incapacitation; finish or expiry must not count twice.
- Preserve record-before-install semantics and existing slow-recipient retirement, message and byte budgets.
- Default bot population remains opt-in. No services, accounts, deployment, game launches or release actions.

## Review focus

- A low slot reused by another allocation must never receive its predecessor's stat row.
- A partial snapshot backlog must defer stats until creation, without waiting for unrelated later frames.
- Recorder refusal must preserve counters, queue offsets, sequences and publication state.
- A late join receives latest absolute totals, without duplicating counts.
- Respawn preserves allocation totals but stale life hits cannot change them.

## Task 1: Recorded accounting (Nextscope)

Files: `gameplay/src/bot_firearm_kd.rs`, `bot_registry.rs`, `bot_firearms.rs`, `events/{combat,statistics,mod}.rs`, focused tests.

- [ ] Add compiling counter/allocation types and failing behavioral tests through the real human Hit and bot scheduler routes.
- [ ] Stage exact live opposing bot kills and all accepted bot victim deaths; keep human collapse/death statistics on their existing path.
- [ ] Encode payloads before recording and install only after accepted damage; verify refusal, stale life, incap/finish, friendly victim death, respawn, reuse and round reset.

Produces `WorldBot.allocation_generation:u32`, `firearm_kd:Totals {kills:i32,deaths:i32}` and registry `stats_publication:State`. Snapshot enqueue calls `State::register(&str,u16,u32,u64)` only after actual append; retirement calls `retire(u16)` and world reset clears state.

## Task 2: Ordered publication (root)

Files: `gameplay/src/bot_stat_publication.rs`, `multiplayer.rs`, `admission.rs`, module/Active seams in `lib.rs`, focused publication tests.

- [ ] Track successfully recorded shared FIFO frames consumed by each reservation, including all real pop paths.
- [ ] Write tests reproducing creation-before-stat, partial drain, continuous unrelated backlog, late join, reuse and recorder refusal.
- [ ] Extend the existing round-robin drain with eligible latest absolute bot stat pairs. Each pair is unsequenced, shares budgets and records before publication-state installation. Derive totals from the current allocation; never enqueue raw application messages into replication FIFO.
- [ ] Preserve existing reliable sequences, NAK history and slow-recipient behavior; run focused publication regressions.

Consumes allocation/counter fields from Task1. State stores recipient/allocation creation positions and last successfully sent absolute totals. `Active.shared_frames_drained:u64` starts at zero and advances on actual recorded FIFO consumption.

## Task 3: Connected proof and integration

- [ ] Replay real external authored bot/combat inputs through ordinary engine admission and firearm paths; bind source/input hashes and scope limitations privately.
- [ ] Run affected checks, full Rust suite, fmt, strict all-target Clippy, build and public-tree boundary once on the frozen combined tree.
- [ ] Publish focused PR; integrate after required CI success and exact merged-tree verification. Preserve release candidate/manual released pin.
- [ ] Record private evidence and advance to the next qualified gameplay TODO without another prompt.

## Decisions

Absolute stat10/stat12 are live scoreboard updates. This work does not claim native scoreboard rendering, application ACK delivery or final bot result rows. Generation plus current engine/round context fences allocation identity; no account surrogate is introduced. Resupply admission and amount remain a separate qualification task.
