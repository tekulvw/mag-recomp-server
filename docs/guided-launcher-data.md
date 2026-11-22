# Guided infantry launcher data

The owner-side projector binds the three Skill 25 launcher purchases to their
required compatible sights, ammo aliases, and original extracted availability
tables. It reads the original game archive, original executable, extracted
catalogs, and independently qualified motion and network receipts. Generated
data belongs in the owner's private runtime inputs.

Run the projector with explicit file paths:

```console
python3 tools/extraction/project-retail-guided-launchers.py \
  --elf "$RETAIL_ELF" --game-archive "$RETAIL_GAME_ARCHIVE" \
  --item-catalog "$RETAIL_ITEMS" --item-bindings "$RETAIL_BINDINGS" \
  --bootstrap-loadouts "$RETAIL_BOOTSTRAP" --skill-purchases "$RETAIL_SKILLS" \
  --armory "$RETAIL_ARMORY" --motion-qualification "$GUIDED_MOTION_PROOF" \
  --ordered-motion-qualification "$GUIDED_ORDERED_PROOF" \
  --network-qualification "$GUIDED_NETWORK_PROOF" \
  --output "$GUIDED_RULES" --proof-output "$GUIDED_PROOF"
```

Add `--check` to reproduce existing outputs byte-for-byte. The checked source
pins include the original three independent evidence receipts. The companion
proof binds the selected archive members, item definitions, alias and Armory
inputs to the exact rules bytes.

The separate `mag_retail_guided_launchers_v1` schema carries guided and no-lock
steering parameters, finite inventory, capsule geometry, authored active
lifetime, generic age allowance and admitted radial damage lanes. The guided
maximum range comes from the global weapon table. The no-lock maximum comes
from each ammunition definition. The `retail_constructor_seed` anchor policy
uses the verified Ground, Helo and Hospital constructor fields. It does not
claim later native property overwrites or substitute the authored vehicle
placement offset. The existing stock launcher projector and catalog format are
unchanged.
