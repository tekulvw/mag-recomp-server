# Domination APC driver implementation plan

> Continue the approved autonomous gameplay-first TODO loop with Sol6.2 Rust and private qualification agents. Source integration after focused checks; releases remain manual after human live-match review.

**Goal:** Make source-qualified attacker APCs driveable and support manual passenger boarding across all six retail Domination profiles.
**Architecture:** Opt in through exact external ground catalogs. Reuse existing authenticated family1 seating, exclusive driver ownership, sparse pose/control relay, dynamic-body updates, current-pose exits and lifecycle revocation. Extend maintained extraction from original sources, then regenerate exact seat visibility and lifecycle/config bindings.
**Tech stack:** Rust1.99, Python extractors and private original-file/compiled-world verification.
**Baseline:** freshly fetched merged main387c4e7ea6137be18e0b4ac09b695d97c6a7f593. SDK/private main ancestry refreshed; unrelated files/dirty native submodule preserved.

## Source facts and policy

Original qualification binds33 spans,28 words,16 branches and all three archive property variants. Type3 APCs use family1 and the common GroundVehicle owner dynamics path. Physical seat0 is operator,1 is turret,2–9 are passengers; authored MaxOccupancy is8. Source allocations place APCs0–7 on two maps and8–15 on the third; never infer kind from index. The literal server allocator-to-vptr store remains untraced; independent original source/class/body and concrete client update-vtable paths establish this bounded shared wire path.

Retain the current conservative all-occupant interpretation of cap8. Reuse explicit replacement policy:8m manual mount range,750ms position age,25m/s owner speed, exclusive driver grants and server-owner0x105 revocation. These values and handoff restrictions are not claimed as exact retail rules. Driver opt-in also enables the same verified manual passenger route. Type7 sources and turret seat1 remain excluded.

## Tasks

- [x] Add Rust REDs for optional exact8 type3/seat0 driver inventory, both allocation ranges, wrongkind/seat/partial/missingpolicy rejection; retain old no-driver Domination and Acquisition type9 behavior.
- [x] Permit only that qualified inventory; reuse normal family1 authority without widening unsupported roles. Exercise connected grant/motion/peer/latejoin, current-pose exits and blocked exits, passenger reinforcement, stale/nonowner denial, revocation and record rollback with focused shared fixtures.
- [x] Add extractor REDs, then an explicit driver opt-in requiring original game archive/registry/faction-property verification. Preserve unflagged spawn/ground bytes. Produce driver rows and manual limits only for source type3; reproject exact visibility including operator0 and no turret1.
- [x] Reproduce six private profiles from current OIC manifests, regenerate lifecycle hashes, keep current collision worlds/active-ability sidecars consistent. Historical core worlds have different hashes and must not silently replace them.
- [x] Validate all48 stationary APC exits against current full dynamic snapshot. On each map prove translated and yawed poses through authenticated owner reports and normal dismount, plus blocked-exit retention. Verify source-bound fullstart and lifecycle alongside these checks.
- [ ] Run affected/full critical checks,fmt/strictClippy/build/public boundary. Integrate focused public PR after green CI and exact merge-tree proof; checkpoint private TODO/evidence with released pin unchanged and advance the next gameplay item without a new prompt.
