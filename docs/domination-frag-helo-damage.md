# Domination Helo frag damage

A Domination Helo configuration can supply `core.helo.frag_damage_areas` to
enable source-qualified ordinary-frag damage. The path is optional and absent
for existing profiles. It works independently of Character and Ground frag
options, hospital rockets and projectile contacts. Current Domination world,
dynamic roster, retail item, material, Helo model and damage inputs are still
required.

The existing `tools/extraction/project-retail-hospital-rocket-helo-areas.py`
produces the strict three-faction geometry catalog accepted by this option.
Its schema name is retained for compatibility. The data binds the original
executable, Main archive, Helo models, damage input and operational collision
packs; it contains model-area thresholds rather than a weapon damage policy.
The separate rocket option retains its rocket prerequisites.

The authenticated ordinary-frag owner path plans Helo effects from the accepted
owner and Ground motion scene before applying Ground health changes. It uses
the current operational Helo mesh, model origin and basis, classified area1–6,
the frag's vehicle dose and authored material penetration, zero prevention
reductions, native direction encoding and existing armor/friendly scaling.
The legacy Helo path already used one material ray; its numerical and source
metadata differences are corrected only when this option is enabled.

Multiple admitted detonations in one owner event apply in order to one fleet
candidate. Each dose is audited and each changed carrier publishes its final
field18 once. The outer event records before installing fleet health, projectile
terminal and queues. A typed query failure records a denial without invoking
legacy Helo damage. Accepted terminal and independent effects survive missing
queries in the opted-in path. The existing flight damage eligibility, minimum
health floors and subsequent critical/death/recovery lifecycle remain in use.

Typed Helo damage deltas require a recipient in the same world with initialized
objects, readiness and Helo creation already sent. A later creation carries
current health. The existing ordinary-frag reliable-queue policy remains:
capacity is bounded, a full eligible recipient becomes backlogged and is retired
through recorded lifecycle cleanup, while healthy recipients continue. This is
separate from hospital rocket retry behavior. Turret-aim queue accounting uses
the actual Helo recipient eligibility.

One dose per carrier per terminal is an explicit bounded replacement policy;
native repeated callbacks remain unqualified. The outgoing vehicle source word
uses the explicit invalid sentinel and the admitted projectile stays in audit.
The native vehicle source input remains unqualified. Original-input projection
and scripted Engine replays establish source behavior; native grenade flight,
callback ordering and live-match acceptance remain separate. Releases are manual
after the maintainer's live-match review.
