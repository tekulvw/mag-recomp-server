# Retail gas grenade data

The offline projector derives the three faction gas grenade rows from an owner's original retail files and reviewed source qualification receipts. It verifies the original Skill 50 unlock, each faction's ItemList and Armory binding, the ordinary grenade equipment mode, and the original projectile ammo token. The rules contain the finite initial count, the fuse and projectile lifetime, and the separate volume duration, radii, and pulse parameters. See [runtime configuration and behavior](poison-grenades.md) to opt in after projection.

Use Python 3.10 or newer from the source checkout. Put both outputs in a private data directory.

```sh
python3 -B tools/extraction/project-retail-gas-grenades.py \
  --elf "$OWNED_RETAIL_ELF" \
  --game-archive "$OWNED_GAME_PSARC" \
  --item-catalog "$DATA/retail-items.json" \
  --item-bindings "$DATA/retail-item-bindings.json" \
  --bootstrap-loadouts "$DATA/retail-all-factions-bootstrap-loadouts.json" \
  --skill-purchases "$DATA/retail-skill-purchases.json" \
  --armory "$DATA/retail-armory.json" \
  --network-qualification "$QUALIFICATION/gas-network.json" \
  --volume-qualification "$QUALIFICATION/gas-volume.json" \
  --output "$DATA/retail-gas-grenade-rules.json" \
  --proof-output "$DATA/retail-gas-grenade-projection-receipt.json"
```

The two qualification inputs are the reviewed retail network and simulation receipts. The projector requires their exact digests and checks the original game executable, authored tables, and data bindings against them. The simulation receipt supplies the default scales and pulse interval. The ammo table supplies the other volume parameters. The generated receipt records the source joins and binds the rules by SHA-256. Rerun the same command with `--check` to verify existing outputs byte for byte.

Generating the file neither changes a player's skill ownership nor enables gas on a running worker. The generated rules, receipts, and original files belong in the owner's private data.
