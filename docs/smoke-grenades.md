# Retail stock smoke grenades

The offline projector reads an owner's original retail files and the retained,
attested all-faction item binding. It selects gear 4 from each stock Marksman
kit, verifies its faction and original ItemList row, resolves the original ammo
token and checks the complete weapon/ammo definitions against their source
archive members. It requires finite grenade inventory, Thrown/Timer/Volume/Smoke
strategies, zero damage in every channel and disabled objective destruction and
hearing loss. Projectile flight lifetime and the separate volume duration are
projected independently.

Use Python 3.10 or newer. Supply paths to files you own and have permission to
use; none are bundled in this repository:

```sh
python3 tools/extraction/project-retail-smoke-grenades.py \
  --elf /path/to/original/mage_g.elf \
  --game-archive /path/to/original/game.psarc \
  --item-catalog /path/to/retail-items.json \
  --item-bindings /path/to/retail-item-bindings.json \
  --bootstrap-loadouts /path/to/retail-all-factions-bootstrap-loadouts.json \
  --output /path/to/private/smoke-grenades.json \
  --proof-output /path/to/private/smoke-grenades-proof.json
```

Repeat with `--check` to reproduce the existing output without rewriting it. The
tool rejects input/output aliases. Its proof identifies original inputs, source
members, native instruction spans and each selected weapon/ammo definition.
Keep the generated resource files and proof private.

To enable the source capability, set `smoke_grenades` to the generated rules
path in an Acquisition configuration, or `core.smoke_grenades` in a Domination
configuration. Paths are relative to the containing configuration file. A
direct Sabotage or Suppression worker (no Acquisition configuration and no
`--match-profiles`) takes `--smoke-grenade-rules /absolute/SMOKE.json` instead,
together with `--objective-catalog`, whose source image identifies the
executable the rules were projected from. In the Helm chart, put the file in
the data bundle and add both flags to `data.gameplayExtraArgs`.

A [match-profile registry worker](worker-match-profiles.md) takes the same
`--smoke-grenade-rules /absolute/SMOKE.json` flag for its Sabotage profiles.
The rules are map-independent, like the falling and incapacitation rules, so
one worker-wide file serves every Sabotage row. Each row binds the rules to its
own objective catalog's source image, so no base `--objective-catalog` is
needed, and startup refuses a Sabotage row without one. Startup also refuses
the flag when the registry has no Sabotage row. Acquisition and Domination rows
ignore the flag and keep the rules their own gameplay configuration names; no
row receives both. Selecting a Sabotage row installs the same capability a
direct worker uses, and selecting any other row removes it. In the Helm chart,
put the file in that worker's bundle and add the flag to the worker's
`extraArgs`.

The catalog must match the loaded retail item, binding, bootstrap, archive and
executable provenance. Omit the property or flag to leave support off; a smoke
throw is then refused like any other unconfigured item, so no peer sees it.
Ordinary frag configuration remains separate. `--validate-catalogs` reports
whether configured smoke profiles were validated, without opening listeners.

A saved kit can hold a throwable in either equipment gear, 4 or 5, and the
launch request names the gear it was thrown from. The server accepts the
normal authenticated launch from either gear for the item equipped there, and
consumes that gear's finite inventory once. Each gear keeps its own count,
seeded from its own item at spawn and refilled by static supply. Accepted owner state 2 or 4 latches the
terminal; the authored fuse and a reported terminal position gate volume
creation. Peer presentation receives state 2. The thrower already executed the
local Timer/Volume effect, and does not receive that visual trigger again; it
still receives the volume and both removals. Smoke never enters the frag damage
planners. Static supply uses the same item/faction capability and inventory cap
as launch.

Up to 32 volumes can be active, with duration measured from explosion. An
exploded parent remains available to late-join snapshots until that deadline,
including after the owner's death. Expiry and reservation teardown queue both
object removals. Reuse waits for pending removals to enter reliable recipient
history. Pool pressure defers a terminal without consuming more inventory; an
unresolved terminal reaching its flight lifetime retires without creating a
volume. Missing position data never substitutes the original launch position.
One-time smoke bootstrap has a separate bounded 96-packet allowance so it does
not exhaust the ordinary reliable queue in a large match.

The retail volume uses family 5, with active, server-owner and position fields.
Its wire row carries no ammo or remaining-duration field. The separate projectile
replication path invokes the client Volume strategy. Server lifecycle tests and
codec fixtures do not establish rendered smoke, exact late-join visual age or
visibility/occlusion behavior. Those require native live-match review before a
human release.
