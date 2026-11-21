# Domination ordinary frag Ground correction implementation plan

**Goal:** Correct ordinary frag damage to Domination APC and motor-pool vehicles through the same recorded, current-scene gameplay path as Acquisition.

**Architecture:** Extend the strict source-area reader with explicit mode selection. Domination accepts exactly six role/faction variants and eight APC/eight motor-pool placements. Its optional `core.frag_ground_areas` owns the rules; the shared planner selects the installed mode's collision and damage policy. Existing absent-option behavior stays intact.

**Scope:** Source gameplay and private original-input verification. Preserve unrelated state, source history, all prior proofs, released pin and manual releases. Current retail proof establishes vehicle dose, zero reductions, model areas1–6, current origin and single layer24 material ray. One dose/key/terminal and MAX outgoing source word remain explicit replacement policies. Native physics, repeated callback parity and live-match acceptance remain separate.

## Source catalog and maintained projection

- Extend `gameplay/src/acquisition_frag_ground.rs` with a closed `Scope::Acquisition|Domination` parser and exact 9/6-role cross-products. Keep the 16KiB bound and reject cross-mode schemas/placements.
- Add optional `--mode domination` to the maintained area projector, binding the existing original executable/archive, six selected chassis and wheel proof. Default Acquisition command/output/proof stays byte-identical.
- Test schema, source, placement, topology and projector rejection. Stage six original Domination profiles and reproduce their selector3/7 wheel proof.

## Runtime and configuration

- Add `State.frag_ground_areas` and load `core.frag_ground_areas` in `domination_gameplay_config.rs`, binding Ground/Objective native ELF, authored Main, dynamic catalog and exact placement roles.
- Supply `World::has_frag_ground_areas`, `validate_frag_ground(items,collision,map)` and `validate_frag_ground_dynamic(catalog,bindings,ground)` for fixed Engine and registry startup.
- Extend current mode selection in `acquisition_frag_ground_damage.rs` and `events/owner.rs`; reuse staged owner/Ground snapshot, current model anchor, native vehicle-only math, material ray, field18 and normal lifecycle. Typed failures preserve legitimate terminal/motion handling and cannot fall through to legacy math.
- Hoist the existing Helo frag call outside the legacy Ground branch under the same admitted/fuse/policy guard. Preserve Helo damage and publication behavior for this increment.
- Pin corrected APC/motor-pool damage, current accepted motion, cover, recorder refusal and absent-option legacy behavior. Watch a meaningful regression fail before enabling the new path.

## Root integration and acceptance

- Wire the World validation methods through `match_profiles.rs`; extend existing readiness aggregation without adding a CLI flag or Engine boolean. Direct Engine setup performs equivalent validation.
- Document optional configuration and explicit limits in `docs/domination-frag-ground-damage.md`.
- Replay all six source profiles with authenticated ordinary frag launch/fuse, independent field18/native arithmetic, terminal/inactive/reuse and separate refused-record case. Include Character/contact-disabled independence and current with-Helo startup/preservation.
- Freeze sources and private inputs, verify their hashes, run the critical affected tests then final fmt/strict Clippy/build/Rust/Python/public-boundary checks. Publish the focused PR and integrate only the checked head after required CI. Checkpoint private evidence and proceed to the next core gameplay item without another prompt.
