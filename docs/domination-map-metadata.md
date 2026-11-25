# Retail Domination map metadata

Generate the metadata locally from a retail disc dump and decrypted
`game.psarc` that you own or have permission to use. Use a fresh output
file outside both the disc dump and this source checkout.

```sh
python3 -B tools/extraction/extract-retail-domination.py \
  --disc /path/to/retail-dump \
  --game-archive /path/to/game.psarc \
  --output /path/to/private/retail-domination-map-metadata.json
```

Repeat with `--check` to compare the output byte for byte against the inputs.
The extractor requires all three maps listed under `GameType256` in the
supported retail settings table. It reads the global rules and the actual
level archives, then checks that spawn, objective, trigger, and callback
metadata refer to the same level and CLV. Missing archives, duplicate spawn
or squad slot identities, and dangling forward spawn references fail extraction.

The version 1 output carries:

- Authored team score and spawn settings, original player bounds, and the
  settings member and archive hashes.
- Map ownership, all authored spawn points and ordered squad links for both
  sides. Special insertion and vehicle points retain their authored types.
- Bunker placement and exit candidates, directly placed objective definitions
  and instances, forward spawn links, and trigger boxes. Objective definitions
  include selected gameplay fields and script hook tokens.
- Immediate and delayed server callback registrations, ordered parameters,
  referenced archive member hashes, and whether an objective hook resolves to
  a placed callback. Lua is read only for hashing.

This is an authored metadata format for future runtime projection. It has no
worker profile, terrain-snapped positions, qualified bunker exits, native
pool or trigger indices, or admission policy. An unresolved script hook is
preserved with `binding_present: false` for follow-up.

## Retail evidence and current server limits

The original settings identify Absheron Refinery, Flores Basin Transfer, and
Alyeska Terminus as the three Domination maps. Each produces 112 spawn points,
32 squad groups, 16 bunker candidates, and 36 objective-to-forward-spawn links.
The authored score starts at 1800/1000 and the squad spawn waves are 20 seconds
on both sides. The authored minimum of 204 and maximum of 256 describe the
original mode. The spawn catalog projector writes a 64-character world in
squads of 8 by default, or this authored 256 with `--capacity authored`; see
[spawn catalog capacity](extraction.md#spawn-catalog-capacity).

The retail ELF independently confirms enum value 2 in the `kGameType_256`
registration and the `GameType256` enum lookup. The retail deploy UI maps its
selection index 0 to that constant. The enum lookup and shared join/setter and
GameInfo codec paths were cross-checked against the retained retail evidence.
Debug symbols were used to identify functions in the shared path. The metadata
tool takes its values and bindings exclusively from retail archives.

Each map places one original primary and four subprimary `ObjectiveControl`
callbacks. The referenced retail subprimary script includes a team 0 deduction
of one point per second. The primary script includes a variable team 1
deduction on a 15-second interval. The callback metadata records eight delayed
entries per map, and flags four unresolved `DeactivatedScript` hooks in
Alyeska. The active runtime graph, callback order, clock epoch, and match-end
conditions still need to be established.

The existing mode 2 server paths use a prototype Alyeska profile and derive an
attacker countdown from elapsed time on owner updates. The retail objective
loader currently accepts modes 0 and 1, and automatic team assignment rejects
bunker-enabled profiles. The next runtime dependency is binding start state,
squads, and objective, spawn, and bunker identities to the retail replication
pools. The extractor can be run without a client, worker, or debugger.
