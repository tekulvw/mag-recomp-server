# Bot-only static resupply implementation plan

> **For agentic workers:** Continue the authorized autonomous source loop with parallel file owners, focused RED/GREEN checks and one Cargo lease. No repeated approval gate or release action.

**Goal:** A dry idle bot explicitly starts at a qualified static supply provider, waits for an authored pulse, receives finite reserve, reloads normally and can fire again.

**Architecture:** The optional source catalog binds internal provider keys, native-oriented trigger geometry and authored cadence to exact runtime spawn/collision inputs. Bot sessions remain independent of firearm reset state and capture world/round/allocation/life/equipment and physical anchor. The firearm scheduler observes fresh quiet/perception state, then stages Start/Cancel/Pulse after all same-tick shots so a later combat hit can cancel before a refill. Recorded candidates install session/reserve changes; existing reload and publication handle subsequent fire.

**Tech stack:** Existing Rust bot/physics/collision/weapon engine, Python standard-library maintained extraction helpers, external owned retail files. No dependencies.

**Spec:** Source-qualified bot-only contract and original static trigger/session/timer proofs retained privately. The initial public slice supports the qualified Cosmodrome Sabotage profile only.

## Constraints and decisions

- Default-off: explicit source catalog, current retail mode0, installed firearm-enabled bot population and exact spawn/collision digests are required.
- ProviderKey is source placement identity, never a fabricated native020b target. Human manual-use interoperability remains separate.
- Own selector: visible, unfrozen and unsnapped placements form an explicit replacement allowlist, without claiming native activation semantics.
- Use authored nonuniform scales and Rz*Ry*Rx axes, not compiled uniform render scale. Inclusive tolerance and tick are explicit runtime data. Geometry parity does not claim PPC SIMD low-bit rounding.
- AI chooses Start only fully dry, idle, grounded and free of conflicting objective/traffic/drop/seat work. Perception deferral is not fresh quiet. Duplicate Start preserves the accepted timer.
- Movement commands are paused while servicing; idle ground/collision physics remains active. Cancel on motion, trigger exit, fresh visible threat, health loss, equipment/life/round/allocation/provider changes. These are explicit replacement interruption policies.
- One due pulse restores only the existing finite bot provisioning reserve cap. No magazine, ammo_pool selector, human reward, UI-state or native use packet is fabricated. Next normal tick reloads.
- First slice starts only at a provider the bot can already reach/use. New navigation goals and route claims remain separate.
- Record before installing Start/Cancel/Pulse. Preserve all unrelated source, accounts, services, native sessions and manual releases.

## Review focus

- Exact source bindings and trigger axes/boundaries must reject stale/mismatched catalogs.
- Early and duplicate service checks cannot refill or restart the deadline.
- A later same-tick shot must interrupt a due pulse before reserve installation.
- Respawn, slot/equipment change and physics movement cannot preserve an old session.
- Recorder refusal preserves session, ammunition and outgoing queues.

## Task1: Catalog/projector — Area

Files: `gameplay/src/bot_resupply_catalog.rs`, `tools/project-retail-bot-resupply.py`, focused Python tests.

- [ ] TDD strict catalog validation, scalar geometry boundaries and projection from maintained original CLV parser.
- [ ] Bind source archive/CLV/instance/definition, explicit tolerance/tick, source-key uniqueness and runtime spawn/collision digests.
- [ ] Reproduce actual private three-provider catalog and receipt; run catalog Rust and focused Python checks.

APIs: Catalog::load, validate_binding(app,map,mode:i8,spawn_sha,collision_sha), select(anchor), provider(key); Provider::contains(anchor), tick_rate_seconds().

## Task2: Recorded bot service — Nextscope

Files: new `gameplay/src/bot_resupply.rs`, `bot_registry.rs`, `bot_firearms.rs`, minimal objective/traffic command gates and focused tests.

- [ ] Observe real scheduler dry→due-refill missing behavior RED, then implement detached Session/Plan and builder.
- [ ] Integrate fresh admission, tail-of-shot interruption, paused commands/current idle physics, exact binding/lifecycle cancellation and record-before-install.
- [ ] Verify dry→Start→early unchanged→due reserve→normal reload→shot; threats, interruption, mismatch, duplicate, refusal and absent config.

Interfaces: Engine.bot_resupply:State containing optional catalog; WorldBot optional service session. Builder `with_bot_resupply_catalog(Catalog)->Result<Self,&'static str>` after installed bot population.

## Task3: CLI, source replay and integration — root/Driver

Files: root `gameplay/src/{lib,resources,main}.rs`, docs; Driver private source replay only.

- [ ] Add explicit --bot-resupply-catalog pre-log validation and configured population/profile/resource requirements; verify meaningful CLI RED/GREEN.
- [ ] Probe grounded membership at actual source provider, then replay the complete service/reload/fire chain and negatives with retained original equipment/map data. Scripted initial placement is explicit; no new route/native-client claim.
- [ ] Freeze source/input manifests; run affected checks plus full Rust/Python, fmt, strict Clippy, build and public boundary.
- [ ] Publish/integrate focused source PR after all required CI and exact merge-tree verification. Preserve release candidate and released pin; checkpoint private evidence and continue the next qualified core TODO.

## Startup stack regression

The full suite reproduced a stack overflow in the existing offline registry-validation CLI test after the additional by-value Engine configuration. The CLI now uses an in-place configure method, while the builder API remains available. The isolated test passes on the standard test stack; no stack-limit workaround was added. All full Rust checks were rerun after this correction.
