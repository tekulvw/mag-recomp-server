# Retail static supply provider data

Owners can project static supply points from their BCUS98110 01.00 ELF and the original level archive. Supply the selected map’s source-bound spawn catalog and matching collision-world file. Run once for each faction profile you want to bind.

| Mode | Supported maps |
| --- | --- |
| Sabotage | Cosmodrome, Kennicott, Remote Comms |
| Acquisition | Alaska Subpen, Panama, VOZ |
| Domination | Baku, Canal, Unalaska |

```sh
python3 -B tools/project-retail-human-resupply.py \
  --elf "$RETAIL_ELF" \
  --level-archive "$RETAIL_LEVEL_ARCHIVE" \
  --spawn-catalog "$SPAWN_CATALOG" \
  --collision-world "$COLLISION_WORLD" \
  --out "$PRIVATE_DATA/static-supply.json" \
  --receipt "$PRIVATE_DATA/static-supply.receipt.json"
```

Use new output paths outside the source checkout. The receipt is optional and lists the original ELF span digests and the output digest. Add `--check` to verify the same output byte-for-byte without writing files. Keep the projected files with the owner’s runtime resources.

The catalog binds the exact supplied spawn and collision files by SHA-256 and records the original client, level archive and CLV fingerprints. Acquisition spawn catalogs carry the original ELF digest directly. Domination spawn catalogs carry the qualified native spawn witness digest. Sabotage spawn catalogs carry the original rules archive digest, and the worker binds the ELF through the profile's objective catalog. Each provider has a packed `020b` interaction target, its low16 slot and source-instance witnesses. The original load model uses the compiled prefab tree, the LevelLoaded callback order and the first clear slot in each fresh family-42 pool. The projection assumes successful creation with the original factories, empty pools at load and no unrelated family-42 allocations during the initial callbacks.

Each provider’s box carries its authored center, three oriented axes, full axis scales and the original Start tolerance. The half extent along each axis is its full scale times `unit_half_extent`. Apply `start_tolerance_m` along each box axis for an inclusive Start range test. `tick_rate_seconds` and `percent_refresh_rate` preserve independent authored properties. Refill counts and caps belong to the equipped item and refill logic. The catalog describes the original source model and supplies external data for a later gameplay consumer.
