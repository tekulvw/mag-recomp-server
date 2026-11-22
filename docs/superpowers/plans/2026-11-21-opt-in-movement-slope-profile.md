# Opt-in authored movement slope profile

**Goal** Make the original movement slope values selectable for navigation bakes and replacement bot physics without altering existing configurations or granting authority to a reference mesh.

**Evidence** In the retained Unalaska reference geometry, the two opening attacker starts on component 61 disconnect at six-cell erosion under a 45-degree slope. Changing only the triangle slope to the original 54-degree value preserves both routes through erosion and region selection. Independently changing only the replacement body's slope lets both exact starts reach the assigned capture volume through the original steering controls. The original archive and retail instruction trace bind both the ascent and no-slide thresholds. Existing body dimensions, step, gravity, speed, and clearance remain fixed.

## Implementation

- [x] Add a maintained source projector and strict Rust loader for an explicit retail movement profile. Preserve the original ELF, game archive, and character member digests. Accept the exact qualified projected data and reject altered bytes, unsupported source, inconsistent angle pairs, unknown keys, and out-of-domain angles. Start with behavioral tests of the CLI and source admission.
- [x] Add explicit movement-profile flags to the existing bake, reference bake, query and inspection commands. Supply only the selected slope through the C++ interface, keeping rasterization, clipping, budgets and all other bake parameters fixed. Version the new runtime and reference artifacts and their native containers. Bind each new manifest to the selected profile file's SHA256 and require the same verified external profile when loading. Preserve the original entry points and the serialization and behavior of their old artifacts.
- [x] Enable explicit profile paths in normal worker startup and runtime registry resource loading, preserving the independent collision-world and physics-region checks. Reject missing or wrong rules, profile/format mismatches, scope substitution, and legacy drop certificates in the new walk-only profile. Keep the opaque reference type outside runtime installation.
- [x] Pass the active runtime mesh's body policy consistently through spawn preparation, allocation, respawn, idle physics, direct movement, nav following, recovery and physics-dependent candidate support. Keep body dimensions and all unrelated movement rules unchanged. Check slope use at the actual physics operations, and preserve default behavior for callers with no new profile.
- [x] Verify a synthetic supported approach separates at the old angle and reaches at the opt-in angle. Pin reference/runtime, profile data, container version and drop rejections. Verify actual body support, respawn, idle and routed movement retain one selected policy. Replay the existing navigation fixtures with default settings.
- [x] Generate a new private Unalaska reference bundle from the retained full geometry using the explicit profile. Verify the Detour result for the two assigned objective 16 approaches and retain the old bundle/report. Run the production movement path where a bounded private static trial can establish it, with no deployment or runtime-coverage claim for the reference map. Record exact source, asset and executable digests, focused/full checks, and the remaining coverage limitation.

The current supported profile file is projected by tools/navmesh/project-retail-movement-profile.py. Profile data is external. The selected version describes the replacement runner's rules with one authored angle change. It does not describe the original native capsule or a complete current collision scene.

## Verification

The separate Unalaska reference bundle returned complete Detour routes for all
18 admitted samples from insertion points 16 and 19 to the objective 16
interaction box. The same requests were disconnected in the prior bundle. All
six original reference bundles retained their manifest and tile data hashes.

The synthetic runtime regression exercises allocation, idle support, routed
walking, death, and respawn with one selected policy. The private all-face trial
also exercised the production follower along each of the 18 Unalaska routes.
Both starts reached a shared shoulder after 36–38 m of physical movement,
departed ground, and stopped with `bot_navmesh_off_corridor`. The original
reference world lacks verified runtime coverage. The private trial neither
qualifies that coverage nor establishes a complete physical objective approach.
