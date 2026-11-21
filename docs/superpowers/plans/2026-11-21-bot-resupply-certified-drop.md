# Bot resupply through one certified drop implementation plan

> **For agentic workers:** Use the existing parallel owners and executing-plans workflow. Standing authorization permits focused implementation and integration without another proposal gate.

**Goal:** An explicitly opted-in autonomous dry bot uses one existing certified drop on a complete route to a supply provider, safely clears the passage, then walks to actual grounded service.

**Architecture:** Extend the existing optional approach policy with one externally selected `allowed_drop_id` and paired bounded `planning_window_ms`, absent by default. Preserve one opaque supply parent across trusted movement children and reuse the existing passage queue, Permit, physical traversal and clearance controller. Separate passage obligations from grounded service admission; a latched interruption prevents service and defers teardown until safe release.

**Tech stack:** Existing Rust engine, source-bound Recast/Detour routes and certified drop controller, Python policy projection, synthetic tests and private source-map offline replay.

**Spec:** `docs/bot-static-resupply.md` and the configuration, ownership and acceptance constraints below. Original-map route/certificate data remains external private input.

## Global constraints

- Walking policy absence and existing service behavior remain unchanged. No original map IDs, coordinates or generated catalogs in public fixtures.
- Require autonomous population, existing objective/drop policy, current fall rules and a certificate on the same pinned mesh for the explicit drop option.
- Exactly one selected DropStart/DropEnd pair; complete, untruncated, bounded routes only. Every walking/drop/continuation query consumes the existing small supply query budget.
- A one-credit policy must make progress: alternate failed walking and enabled drop attempts on the same candidate across retries.
- Preserve Task role, objective, role_goal and normal controller ownership. No synthetic objective, human XP or objective delay learning for supply movement.
- Use existing Permit and physical sweeps. Queue-frozen Complete and projected endpoints never authorize service.
- An interruption after physical commitment retains safe motion/egress and ownership until release. Before commitment, cancellation suppresses launch and freezes the old owned prefix before movement can resume.
- Record new supply transitions before installing their owned route/cancel state. Failed validation restores prior motion. Do not widen this task into a legacy queue transaction rewrite.
- Source integration after checks is authorized; game/service/deployment/account changes, released pins and releases remain outside this task.

## Review focus

- A holding/egress child must inherit only its current same-life supply parent; a newer unrelated move survives old seek cleanup.
- A queued frozen pose outside the provider must wait rather than begin service or be mistaken for terminal arrival.
- A latched cancellation during traversal must clear the certificate before teardown and grant no reserve.
- A complete route query is insufficient: actual queue, grant, trajectory, landing, clearance and grounded provider membership must be observed.
- The real scheduler runs tactical admission before firearm observations; the replay must exhibit a real eligible idle window rather than silently taking over a tactical route.

### Task 1: Optional certificate policy and offline dependency checks

**Owner/files:** Root: `bot_resupply_approach_policy.rs`, `tools/project-bot-resupply-approach.py`, projector tests, `main.rs`, docs.

**Interface:** `Policy::allowed_drop_id() -> Option<u32>` and `planning_window() -> Option<Duration>`; positive optional external ID paired with a 1,000–10,000 ms planning window no longer than seek duration; both absent retain old walking output. `validate_binding` verifies the selected link exists in the exact loaded artifact. Engine installation and CLI validate autonomous/drop policy/fall dependencies.

- [x] Add failing loader/projector cases for absent, zero/invalid and selected IDs; implement strict optional positive ID and preserve prior no-option policy bytes.
- [x] Validate current mesh certificate and explicit runtime dependencies before logs; expose the getter to runtime and add relevant synthetic checks.
- [x] Document one-passage scope, query accounting and safe cancellation, keeping raw source data external.

### Task 2: Parent-owned seek and bounded route continuation

**Owner/files:** Next: `bot_resupply_approach.rs`, `bot_resupply.rs`, `development_bot_movement.rs`, `bot_firearms.rs`, focused runtime tests.

**Interfaces:** `Seek::{parent_id,allowed_drop_id,passage_resume_goal,cancellation_pending,owns,passage_released(id,cleared)}`; `Target::set_registry_supply_parent(&str)` for trusted current children; `Engine::prepare_resupply_drop_resume(&self,id,&ActorGrid,now) -> Result<PreparedRegistryMove,&'static str>` for the holding planner after its query credit is charged.

- [x] Write failing cases for exact one-pair validation, child ownership, queued Complete, interrupted traversal and release-before-continuation.
- [x] Retain a source-bound complete route with the exact selected certificate, current HP and existing fall rules; preserve endpoint support and unrelated passage exclusions. Do not grant entry from route selection.
- [x] Fix the source-proved idle350→tactical400 scheduling handoff with a recorded bounded planning reservation admitted only while grounded, idle, fully dry and free of existing holds/traffic/movement. It pauses new planning while normal perception and charged queries resolve. It has no quiet, passage, movement or service authority. Cancel/expire with a retry cooldown that gives tactical planning a real turn; consume only on an accepted complete-route seek. Preserve refusal and current life/equipment/scope. No unrelated route takeover.
- [x] Carry the unique recorded parent on trusted movement children. Split identity/equipment/config checks from strict service grounding. Latch one cancellation reason while an obligation exists, without reopening service.
- [x] After actual same-life clearance, spend a separate bounded credit on a complete walking continuation from the current body to the stored provider endpoint. Abort on an additional required passage. Restore tactical eligibility only after obligation release and final supply teardown.
- [x] Prove recorder refusal, stale allocation/parent, precommit and committed interruption, unchanged ammo and strict actual arrival. Keep fire/reload suppressed throughout the seek.

### Task 3: Existing drop-controller consumers

**Owner/files:** Area: `bot_registry.rs`, `bot_objectives.rs`, narrow objective callback seam only if needed, focused tests.

**Interface:** `BotRegistry::supply_passage_obligation(object_id,generation,drop_id) -> bool`, tied to exact holder/ticket and unsafe physical obligation; consumes Task2 parent and resume APIs.

- [x] Write focused cases for queue admission, competing holder, trusted child propagation and cancellation before launch versus after commitment.
- [x] Admit only the exact owned opted-in supply passage into the existing queue/controller. Preserve permits, collision, convoy/headway and true clearance. Exclude supply from objective delay episodes and reconsideration.
- [x] Permit only the bounded supply holding/recovery lane in the objective scheduler. Resume toward stored supply endpoint without changing Task goal/progress/role; record the supply parent transition and restore scratch route on rejection.
- [x] Notify actual release or terminal abort. Freeze safe canceled preentry motion before the movement pass; keep committed motion/recovery until safe clearance. Leave ordinary objective and walking supply gates intact.

### Task 4: Source-map acceptance and integration

**Owner:** Driver owns private replay; root owns full checks, PR and private TODO checkpoint.

- [x] Use the exact retained insertion, mesh, provider and fall inputs. Seed only dry equipment and normal supported insertion state; run actual scheduler order and retain the eligible admission opportunity.
- [x] Observe walking, queue/grant, certified entry, physical descent/landing, clearance and walking continuation, then actual grounded in-box Start, one due pulse, ordinary reload and shot. No scripted crossing or arrival.
- [x] Run focused regressions, full Rust tests, Python tests, fmt/strict Clippy/build and public boundary with one coordinated Cargo lease. Fix observed failures before expanding checks.
- [ ] Integrate focused PR after six CI checks, compare checked/merged tree, retain private evidence and continue TODOs. Releases stay manual. Local checks passed: 1,195 Rust tests/36 suites, 313 Python tests, fmt, strict Clippy, build and 1,212-file boundary. Exact corrected-source replay independently matched 275 product and 48 input hashes and passed the full insertion-to-service/reload/shot path; five offline startup cases passed.
