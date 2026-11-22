# Projecting retail flash grenade rules

The offline projector derives the three faction Flash items granted by the
original no-cost Recruit row. It validates each grant against its original
faction ItemList, wire binding and default Armory availability. An operator
can equip these items through the normal saved-loadout path. The unavailable
legacy Flash skill is not a purchase requirement.

Pass the owned retail ELF and game archive, matching extracted item and
availability metadata, and the three reviewed gameplay, facing and material
qualification receipts.

```sh
python3 tools/extraction/project-retail-flash-grenades.py \
  --elf "$RETAIL_ELF" --game-archive "$RETAIL_GAME_ARCHIVE" \
  --item-catalog "$ITEM_CATALOG" --item-bindings "$ITEM_BINDINGS" \
  --bootstrap-loadouts "$BOOTSTRAP_LOADOUTS" \
  --skill-purchases "$SKILL_CATALOG" --armory "$ARMORY" \
  --gameplay-qualification "$FLASH_GAMEPLAY_RECEIPT" \
  --facing-qualification "$FLASH_FACING_RECEIPT" \
  --material-qualification "$FLASH_MATERIAL_RECEIPT" \
  --output "$FLASH_RULES" --proof-output "$FLASH_PROOF"
```

Add `--check` to compare both existing outputs byte-for-byte. The tool
rejects aliased outputs and unreviewed or mismatched receipts. Keep populated
rules, proof and original inputs in the owner workspace.

The `mag_retail_flash_grenades_v1` rules carry the distinct faction item and
ammo bindings, finite use count, fuse, projectile lifetime, radii and material
penetration. Their shared blind values include both authored durations,
falloff, and the native facing, material-filter and message bounds. The
projector validates Thrown/Timer/Debris, the hearing flag and zero damage
channels, then records source and definition digests in the proof. The weak
value is a shorter immediate blind. It does not schedule another effect.

Projection qualifies data only. It does not enable gameplay, validate a
live client session, or project client audiovisual assets.
