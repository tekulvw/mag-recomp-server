# Acquisition projectile contact bindings

Acquisition profiles can optionally provide a mode-1
`mag_acquisition_projectile_contacts_v1` document through the top-level
`projectile_contacts` path in their Acquisition gameplay configuration.
Direct and Registry startup verify the loaded Structural Sequence assets,
retail ELF identity, layer-23 filter row, raw collision world and dynamic
body catalog against the binding. The worker reports
`projectile_contacts_validated` on an offline startup check. The binding
by itself does not admit a launcher or resolve damage.

Given operator-qualified source receipts, generate the binding with:

```sh
python3 tools/extraction/project-retail-acquisition-projectile-contacts.py \
  --map MAP_NAME \
  --qualification /absolute/qualified-contact-inputs.json \
  --filter-receipt /absolute/original-filter-enable-pairs.json \
  --static-world /absolute/world.json \
  --static-receipt /absolute/static-ray-receipt.json \
  --dynamic-catalog /absolute/dynamic-bodies-with-hulks.json \
  --sequence-index /absolute/sequence-assets/index.json \
  --output /absolute/acquisition-projectile-contacts.json
```

Use `--check` to compare an existing output with reproducible bytes.
The qualification input carries the original layer-23 row, raw filter
receipt and dynamic catalog digests, and a map-keyed inventory. Each map
entry includes the static world and receipt digests, Sequence index and
physics member digests, and the Sequence packs with their source and
neutral hashes, root tags and part IDs. Qualification requires the original
Sequence resources and native filter receipts in the operator's custody.
The projector compares the supplied pins and derives the layer-23 row
from the pinned native enable pairs. The worker validates the loaded
runtime resources against the result.

Acquisition contacts combine represented static reference geometry with
current Structural Sequence surfaces and current Character and Ground
bodies. Ground wheel phantoms are excluded. The Acquisition binding accepts
only Structural Sequence overlays, and Domination uses its own binding
scope for Hospital and Helo assets. Reference static transforms do not
establish parity with every live native body.
