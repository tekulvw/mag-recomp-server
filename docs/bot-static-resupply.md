# Bot-only static resupply

The optional Cosmodrome Sabotage provider catalog enables one timed reserve
refill for a dry idle bot at an authored supply point. Load it with
`--bot-resupply-catalog /absolute/SUPPLY.json` alongside a firearm-enabled bot
population. The worker requires retail mode0, the selected source map, and exact
spawn/collision file digests. The option is absent by default.

Project private operator data from the owned original level and current runtime
inputs using the maintained tool:

```sh
python3 tools/project-retail-bot-resupply.py \
  --level-archive /absolute/mag_cosmodrome_final.psarc \
  --spawn-catalog /absolute/SPAWNS.json \
  --collision-world /absolute/WORLD.json \
  --out /absolute/SUPPLY.json \
  --receipt /absolute/SUPPLY.receipt.json
```

Add `--check` to reproduce existing bytes. The first supported allowlist selects
visible, unfrozen, unsnapped source placements. That is an explicit replacement
policy, not a native activation claim. Original files and generated provider
catalogs are external inputs and are never bundled into public source.

The trigger uses authored translation, nonuniform axis scales and the qualified
X/Y/Z rotation order. It does not substitute the compiled render matrix's
uniform scale. Current grounded physical position must satisfy the inclusive
oriented box test. Tolerance and tick interval come from the catalog. The
geometric representation does not claim PPC SIMD rounding parity.

AI service admission requires a current, grounded, fully dry finite firearm,
idle completed motion and fresh quiet perception, without an objective hold,
seat, drop or traffic operation. Merely overlapping a provider does not refill
ammunition. The recorded Start creates a session with an accepted deadline;
repeated checks retain it. New movement commands pause during service while
idle collision/ground physics still runs.

Fresh enemy visibility, recorded health loss, movement, leaving the box or a
changed life, equipment, allocation, round or provider cancels service. The
scheduler evaluates the service after all shots in that pass, so a later
scheduled enemy hit can interrupt a due pulse. A health-loss revision preserves
that interruption even when healing restores HP before the next check.

One admitted due pulse restores the existing finite bot provisioning reserve
cap and closes the session. Magazine, reload state and the ammo_pool selector
remain unchanged by the pulse. The next ordinary scheduler tick starts reload;
normal weapon timing and field11 publication handle the later shot. Start,
Cancel and Pulse record before installation, and recorder refusal preserves
session and ammunition.

This slice services a bot already at a supported source provider. New resupply
navigation goals, human020b interoperability, native packed target identities,
other maps and native animation/presentation remain separate. Internal provider
keys identify exact source instances and are not emitted as native handles.
No human account, resupply XP, UI state or use packet is fabricated. Native
live-match review and release cutting remain the maintainer's responsibility.
