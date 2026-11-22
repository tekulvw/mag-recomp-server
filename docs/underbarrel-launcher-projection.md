# Retail underbarrel launcher projection

Run the projector against owner-provided original retail v1.0 source files and their bound item, bootstrap, skill and Armory projections.

```sh
python3 tools/extraction/project-retail-underbarrel-launchers.py \
  --elf "$RETAIL_ELF" --game-archive "$GAME_ARCHIVE" \
  --item-catalog "$ITEM_CATALOG" --item-bindings "$ITEM_BINDINGS" \
  --bootstrap-loadouts "$BOOTSTRAP_LOADOUTS" \
  --skill-purchases "$SKILL_PURCHASES" --armory "$ARMORY" \
  --output "$UNDERBARREL_RULES" --proof-output "$UNDERBARREL_PROOF"
```

Use `--check` with the same arguments to compare both generated files without changing them. Keep both outputs with the external retail inputs.

The `mag_retail_underbarrel_launchers_v1` rules identify three faction stock AR hosts and one compatible launcher per host. The projector binds each attachment to its own WeaponData, original faction ItemList and skill reward. The proof records retail slot-17 producer instructions, source hashes and the original shared ammo token and alias binding.

Add `--include-tier2-hosts` and choose separate output paths to generate
`mag_retail_underbarrel_launchers_v2`. This adds each faction's tier-two
assault rifle and its compatible launcher after the three stock pairs. The
projector checks the original rifle and launcher skill grants and verifies that
all six launchers share the same ammo definition. Use the same switch with
`--check` for these outputs. The worker also needs ordinary firearm projections
for the three tier-two hosts before it can load the six-pair catalog.

The shared ammo record preserves authored collision, weapon strategy, physical, timing and damage fields. In particular, the impact damage and detonation timer fields are authored values. Their presence alone does not define runtime contact, bounce, damage or timeout semantics. This tool performs offline extraction and adds no gameplay admission.
