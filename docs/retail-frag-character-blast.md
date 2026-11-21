# Ordinary retail frag Character damage

A concrete Domination profile can opt into ordinary on-foot frag damage with
`core.frag_character_blast: true`. The setting is absent/off for existing profiles.
It does not require hospital rocket equipment or the development grenade-radius
switch. Both Engine configuration and match-profile startup require current map
collision, dynamic bodies, the core's material/terrain/Sequence inputs, retail
Character blast rules and grenade authority before enabling it.

The maintained `tools/extraction/project-retail-combat.py` projects authored
`material_penetration` alongside each frag's existing ammo identity, dose, radii
and fuse. Legacy metadata without the field remains usable by existing paths;
the new opt-in rejects missing, negative or nonfinite penetration. Original game
files and extracted catalogs remain external inputs.

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

Blocked-ray native fallback, exposed mounted targets, native projectile physics
and presentation-specific weapon indices require separate qualification.
Synthetic checks and scripted original-data Engine replays establish source
behavior only. Native input and live-match acceptance remain separate, and the
maintainer reviews the live match and cuts releases manually.
