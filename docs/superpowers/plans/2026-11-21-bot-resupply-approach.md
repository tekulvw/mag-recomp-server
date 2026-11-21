# Bot resupply walking approach implementation plan

> **For agentic workers:** Use the existing parallel implementation owners and executing-plans workflow. Standing authorization permits implementation and focused integration without another proposal gate.

**Goal:** A fully dry bot walks a complete short route into a qualified static supply trigger, then uses the existing timed reserve service and ordinary reload/fire.

**Architecture:** An explicit external policy binds finite source-keyed approach anchors to the loaded provider catalog, collision world and Detour payload. The firearm scheduler supplies fresh quiet observations; recorded seek ownership preserves one validated walking corridor through the existing physical follower. Current grounded body membership, rather than route projection, authorizes the separate service Start.

**Tech stack:** Existing Rust gameplay engine, Recast/Detour wrapper, Python standard-library projection tooling and synthetic tests.

**Spec:** The qualified static-service contract in `docs/bot-static-resupply.md`, extended by the approach constraints below. Original-map evidence and physical acceptance remain private external inputs.

## Global constraints

- Optional approach policy absent means existing service-only behavior.
- No game data or extracted coordinates in public fixtures. No new FFI or invented native identities/XP.
- Complete bounded walking routes only; reject partial, truncated and drop-bearing corridors.
- Preserve actual objective holds, tactical roles, allocation/life/equipment and movement ownership.
- Record before seek/service mutation; refusal preserves prior authoritative state.
- Finite per-tick query budget, finite candidate inventory, retry and seek deadline.
- Source PR integration is authorized. Releases, released private gitlink, deployment and live services remain unchanged.

## Review focus

- Projection into a box cannot authorize arrival; tests must inspect actual grounded feet.
- A superseding movement owner must retain its route when an old seek is cancelled.
- Deferred perception grants neither quiet admission nor service.
- Navigation errors consume query credits; repeated failures obey retry and fair caller rotation.
- The CLI must reject missing/mismatched policy dependencies before opening output logs.

### Task 1: Bound policy and maintained projection

**Owner/files:** Area: `gameplay/src/bot_resupply_approach_policy.rs`, narrow catalog digest/key access in `bot_resupply_catalog.rs`, validated `NavMesh::goal_candidates` and `navmesh_cli.rs` goals command; `tools/project-bot-resupply-approach.py` and associated Python tests. Root registers module and worker CLI.

- [x] Pin strict schema: application/map/mode, provider raw SHA, collision SHA, consumed Detour SHA; explicit query credits, retry, goal radius, route length and seek duration; finite unique in-box anchors for exact provider keys.
- [x] Write failing synthetic loader/binding tests for invalid limits, nonfinite/duplicate/outside anchors, missing/extra provider and source mismatch; implement loader and validation.
- [x] Validate existing goals API inputs and returned finite component candidates. Expose bounded query CLI with verified mesh/world binding; no new FFI.
- [x] Project anchors using bounded explicit sampling offsets and the current goals executable, filtering exact provider containment. Test determinism, failure, binding and limits. Keep separate receipt.

### Task 2: Recorded seeking through existing physics

**Owner/files:** Next: `bot_resupply.rs`, new focused seek module if needed, `development_bot_movement.rs`, `bot_registry.rs`, `bot_objectives.rs`, `bot_firearms.rs` and necessary narrow objective callback gates.

- [x] Write failing ownership, route rejection, refusal, perception deferral and actual-arrival tests.
- [x] Configure policy in-place through `Engine::configure_bot_resupply_approach_policy`; require configured supply catalog, current mesh and collision world.
- [x] From actual physical pose and fresh quiet, attempt bounded candidate routes fairly. Validate complete walking path, both projected/final endpoint membership, endpoint support/occupancy, current protected passages and traffic.
- [x] Prepare a retained complete `NavigationMove` without replanning or truncation. Record and install exact allocation/life/equipment/provider/movement token and deadline; preserve tactical goal.
- [x] Gate other planners only for this owned seek. Continue physical movement, damage and observation while suppressing reload/fire. Interrupt on threat, damage, invalid scope, expiry or lost token; clear only owned motion and make normal goal freshly replannable.
- [x] After actual settled grounded arrival and fresh quiet, record the separate existing service Start. Walking time does not count as service. Prove reload/fire resumes after one reserve pulse.

### Task 3: CLI and bounded physical acceptance

**Owner/files:** Root: `main.rs`, `lib.rs`, `docs/bot-static-resupply.md`, resources documentation and CLI tests. Driver: private source-bound physical replay only.

- [x] Add optional absolute `--bot-resupply-approach-policy`; require catalog/mesh/world and validate exact binding before logs, including validate-only startup. Configure in-place after supply setup.
- [x] Test absence, relative path, missing dependencies and invalid binding; document explicit knobs and walking-only scope.
- [x] Build owned current source once under coordinated Cargo lease. In offline original-map replay seed only initial supported outside pose/dry ammo; retain successive actual positions and let existing follower reach the trigger.
- [x] Verify actual body outside before walk, grounded membership before Start, early no refill, exactly one due pulse, normal reload and admitted shot. Verify interruption/refusal without teleport or synthetic arrival.
- [ ] Integrate the focused source PR after six CI checks; compare checked and merged trees, retain private evidence and continue TODOs. Local validation passed: 1,184 Rust tests/36 suites, 312 Python tests, fmt, strict Clippy, build and the public boundary check. The exact formatted-source original-map replay passed physical walk, service, reload and fire; 275 source and 44 input hashes independently matched. Six offline startup checks passed. Integration remains pending CI.
