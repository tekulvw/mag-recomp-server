# Acquisition and Domination reference navigation plan

> **For agentic workers:** Implement and verify each slice in the isolated worktree. Keep actual collision and navigation artifacts private.

**Goal:** Generate and verify whole-map offline navigation for the six current Acquisition and Domination collision inputs.

**Scope:** The inputs describe provisional static reference geometry. The reference mesh has its own opaque query type. Existing verified movement, normal runtime loading, drop certification and existing bundle formats keep their admission rules.

- [x] Add explicit reference input to the maintained geometry preparer. Preserve digest, authored winding, emitted geometry and original region metadata.
- [x] Implement the reference-only bake, manifest/binary sidecar, bounded loader and offline inspect/query commands. Retain the pinned Recast geometry profile. Use a versioned 512-tile, 8,192-polygons-per-tile Detour bundle with a 192 MiB reference byte cap and retain the existing gameplay cap.
- [x] Cull reference tile faces against precomputed triangle X/Z bounds and the exact expanded tile footprint, preserving their original order. Test the old grid limit, scope fences, artifact corruption and cross-tile queries.
- [x] Measure a private full Baku pilot before running at most two map jobs in parallel. Reopen all six bundles, retain checksums, report representative route results and keep original assets external.
