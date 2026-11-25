# Ordinary retail frag Character damage

Concrete Acquisition and Domination profiles can opt into ordinary on-foot frag
damage. Domination uses `core.frag_character_blast: true`; Acquisition uses
`frag_character_blast: true` in its gameplay configuration and requires its
Structural provider. The setting is absent/off for existing profiles. It does not
require hospital rocket equipment or the development grenade-radius switch.
Both direct Engine configuration and match-profile startup require current map
collision, dynamic bodies, source material/terrain/Sequence inputs, retail
Character blast rules and grenade authority before enabling it. Acquisition uses
its current Structural runtime's collision frame rather than a separate copy.

The maintained `tools/extraction/project-retail-combat.py` projects authored
`material_penetration` alongside each frag's existing ammo identity, dose, radii
and fuse. Legacy metadata without the field remains usable by existing paths;
the new opt-in rejects missing, negative or nonfinite penetration. Original game
files and extracted catalogs remain external inputs.

The maintained `tools/extraction/project-retail-active-ability-static-ray.py`
accepts `--materials` with its existing original archive, executable, assembled
world and source-root inventory inputs. It adds the original compiled subpart's
material slot or the terrain marker without changing geometry, filters, coverage
or qualified source omissions. Its no-flag output remains unchanged. Updated
world hashes require matching static-ray receipts and active-ability sidecars.
The optional typed receipt checks the material inventory against the output count.

An enabled Acquisition profile validates every row-24-eligible root/child pair's
static material binding and known cost, plus every slot used by its loaded terrain
grid. Legacy unannotated worlds are rejected before readiness. The native
collision-null slot keeps its existing penetration behavior independently of the
ordinary cost table. No fallback material is assigned.

A launch must already pass normal equipment, ammo and slot admission. The current
owner's authenticated UDP report supplies its terminal position, and the authored
fuse still governs detonation. Reported expiry or an unknown position does not
become a damage source. Retail uses four projectile slots and prototype uses three.
This path does not reconstruct native grenade flight, bounce or contact physics.

The Character collector tests current represented collision bodies against the
blast sphere, uses the current stance-adjusted Character anchor for radial falloff,
and traces one layer-24 material ray. Accepted same-event on-foot position and
stance and staged Ground movement belong to one immutable event view. A staged
snapshot cannot pass the ordinary installed-state snapshot fence. Sequence and
parent-attached reference-pose limitations remain the same as the represented
world used for hospital rocket contacts.

Qualified damage retains the callback's Character armor inputs, native location
and type, encoded direction and the admitted projectile's object identity.
Reservation and life generation bind each queued target. Existing owner-reply
health, incapacitation, statistics and reward handling consume the recorded
pending dose. Armor and team modifiers are captured at detonation; later replies
recheck current reservation, life and mount eligibility and existing downed-life
rules. This is the replacement's established queue timing, not a claim of the
native immediate callback order. The bounded queue withholds excess new doses
and records its capacity denial. The owner event records before installing
terminal, position, queue or damage changes. Configured source handling takes precedence over the
legacy development route, so a terminal cannot apply both paths.

Each qualified ordinary-frag dose carries private server-side provenance:
world, round and, for a registry bot, its exact allocation as well as the target
life. This is produced by the admitted blast path, not a client flag. Queue
drain rejects a stale scope or replacement allocation before applying its
health change. Generic explosive damage and the development-radius path do not
acquire this provenance.

A registered bot's first positive-health-to-zero qualified frag collapse adds
one to its existing absolute Deaths statistic, native stat12. Victim accounting
has independent eligibility from the thrower's team or reward account. The
existing event record precedes health and counter installation, and the shared
creation-fenced statistics drain publishes the current allocation's row. A
same-allocation respawn keeps its totals; replacement allocation starts fresh.
Admitted postterminal damage grace leaves the frozen round counters unchanged.
Existing thrower rewards, assists, death notices and final results retain their
previous scope.

## Sabotage

Sabotage has no gameplay configuration or represented projectile scene: no
dynamic body catalog, collision filter rows or material bindings. Its frags use
the static map world instead. The worker turns this on for a direct retail
Sabotage launch (no match profiles or Acquisition setup) that loads
`--collision-world` for its map, `--character-damage-rules` and retail combat
metadata with grenade damage authority. The `listening` and
`--validate-catalogs` output report `static_world_frag_blast` with the qualified
frag items, or the reason it is off. Owner updates then report the grenade
visibility policy `static_world_stance_anchor_ray` instead of
`unknown_no_damage`.

A `--match-profiles` registry worker qualifies each Sabotage profile the same
way, from that profile's own `collision_world` and `character_damage_rules` and
the worker's retail combat metadata. Selecting a qualified profile turns the
static-world blast on; selecting any other profile turns it off. The per-profile
status appears under `static_world_frag_blast.per_profile` in `listening` and
`--validate-catalogs`, and under `character_weapons.frag_blast` in
`match_profile_selected`. Acquisition and Domination profiles report whether
their represented scene's `frag_character_blast` opt-in is set.

Admission, the authored fuse and the owner's reported terminal position are
unchanged. At detonation, every living on-foot human and registered bot in the
world whose stance anchor lies inside the authored outer radius is a candidate,
including the thrower. The event's own Character uses its accepted same-event
pose; others use their latest tracked position and owner stance. A candidate
takes the authored dose with the native falloff unless a static triangle crosses
the straight line from the blast to its anchor. Any crossing is cover: the
world has no materials to penetrate, so authored material penetration is
neither required nor applied. Body overlap at the outer edge is not modelled;
the anchor distance alone decides range.

The dose enters the same ordinary frag queue as the other modes, with the same
armour, friendly-fire multiplier, provenance, incapacitation, statistics and
reward handling. Vehicles, objectives and mounted Characters take no frag damage
in Sabotage.

Blocked-ray native fallback, exposed mounted targets, native projectile physics
and presentation-specific weapon indices require separate qualification.
Synthetic checks and scripted original-data Engine replays establish source
behavior only. Native input and live-match acceptance remain separate, and the
maintainer reviews the live match and cuts releases manually.
