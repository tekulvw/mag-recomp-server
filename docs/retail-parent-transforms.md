# Retail collision parent attachments

The compiled-world exporter can resolve collision packs attached to named rigid
geometry nodes. This is an explicit **compiled reference pose** export. It does
not reconstruct animation or certify collision at the current game time.

The default export continues to reject parented packs. To export their reference
poses, supply an externally reviewed native witness and opt in:

```sh
python3 tools/collision/retail_compiled.py \
  --archive "$MAP_ARCHIVE" \
  --map-manifest "$MAP_MANIFEST" \
  --instance-join "$INSTANCE_JOIN" \
  --catalog "$NEUTRAL_COLLISION_CATALOG" \
  --terrain-proof "$TERRAIN_WITNESS" \
  --placement-proof "$PLACEMENT_WITNESS" \
  --retail-elf "$RETAIL_ELF" \
  --map-name "$MAP_NAME" \
  --terrain-token "$TERRAIN_TOKEN" \
  --reference-pose \
  --parent-proof "$PARENT_WITNESS" \
  --output "$NEW_OUTPUT_DIRECTORY"
```

Keep all original inputs, extracted catalogs, native-analysis evidence and
generated worlds private. Public builds and synthetic checks require none of
them. The output directory must not already exist. Missing source packs remain
errors; parent support does not omit unresolved geometry.

The exporter covers present compiled collision records. Its `compiled_absent`
and `compiled_absent_with_authored_reference` counts remain visible in the
assembly report. An absent compiled record does not prove that an object has no
entity-created collision, and resolving all present records is not a claim that
every source object or collision lifecycle is represented.

## What the resolver checks

The existing exporter rechecks the original archive, compiled level, regenerated
instance join, resource bindings and neutral mesh hashes. The parent resolver
then reads the geometry resource and compiled transforms from the original
archive. It accepts the supported resource-version-63 rigid hierarchy, nested
renderable version 29 and instance version 7. Unsupported attachment sections,
versions and malformed boundaries are errors.

Each serialized node has a name token, a transform-array index and a parent-node
index. Those indices have different meanings. The resolver validates native
root reachability, child/sibling links, parent relationships, bounds, unique
names and the absence of cycles. It resolves the collision pack's parent token
to a reachable node and follows the actual parent links.

Compact poses contain an xyzw quaternion, xyz translation and uniform scale.
With row vectors, the world matrix is:

```text
selected local matrix * parent local matrix * ... * root local matrix
```

The compiled root participates once. Applying the final compiled collision
placement afterward would apply the root twice. Compact affine matrices also
have homogeneous W=1; the separate scale field in a serialized collision matrix
is not that homogeneous coordinate.

The collision consumer needs a further step. It extracts a quaternion from the
raw world matrix and normalizes the resulting quaternion. It does not first
normalize the matrix basis. The replacement model preserves that order, then
uses the compiled root scalar to scale the collision shape. A nonunit root can
therefore produce a different rotation from a conventional scale-removal step.
Nonunit child scales are rejected because that collision-consumer case is not
covered by the supported contract.

## Native witness contract

The witness is JSON with schema `mag_retail_parent_transforms_v1`, the selected
retail ELF SHA-256, `scope: compiled_reference_pose`, and
`composition: row_vector_child_parent_to_root_once`. Its `functions` list contains
unique records with `name`, hexadecimal `retail_address`, `retail_size` and
`retail_sha256` fields. Required function names are listed in
`tools/collision/retail_parents.py`.

Those records cover the physics parent-token reader, rigid hierarchy readers,
named-node lookup, compact matrix conversion, matrix multiplication, quaternion
extraction and animated collision consumers. The tool checks the complete ELF
identity and every function's file-backed span against the witness. It verifies
the identity of supplied analysis; it does not recreate or independently approve
the semantic native analysis. Supply a reviewed witness for the supported build.

The resource-version-13 physics header's extra resource-level word is the
instance count. It precedes the arrays of payload sizes, material-index counts,
parent tokens and collision tags. Parent tokens are fixed-width words bound to
the corresponding MOPP records, not resource substitutions or extra placements.

## Output and remaining limits

`world.json` remains a provisional neutral triangle world. `assembly.json`
records `scope: compiled_reference_pose` and a `parent_resolution` section with
source header hashes, compiled transform hashes, selected node chains, transform
indices, computed reference matrices and the native-witness identity.

Parented reference-pose exports have **no verified physics regions**. Combining
them with `--physics-region-min` or `--physics-region-max` is rejected. Animated
objects can move immediately, including when their definitions enable playback
on start. A reference pose cannot establish current door, machinery or other
moving collision, bot traversability, or full-map navigation coverage.

The math is a finite scalar float64 model with float32 output vertices. It is
not bit-exact emulation of native SIMD, fused operations or reciprocal-square-root
estimates. Synthetic tests exercise noncommuting transforms, multiple ancestors,
separate node/transform identities, scale handling and malformed source records.
They establish the replacement model's behavior, not an observed game run.
