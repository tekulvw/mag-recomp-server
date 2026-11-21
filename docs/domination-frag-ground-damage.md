# Domination vehicle frag damage

A source-bound Domination configuration can supply `core.frag_ground_areas`
with a path to qualified APC and motor-pool damage-area rules. The option is
absent/off for existing profiles and independent of `frag_character_blast`,
hospital rockets and projectile contacts. Both fixed Engine setup and registry
startup validate current Structural/material, retail item, Ground source and
dynamic-body inputs before enabling it.

The maintained `tools/extraction/project-retail-acquisition-frag-ground-areas.py`
accepts `--mode domination`. That mode emits the distinct
`mag_domination_frag_ground_areas_v1` schema: six variants covering type-3 APC
and type-7 motor-pool vehicles for all three factions. Each profile must bind
eight placements of each role. Acquisition keeps its nine-variant schema and
default projector output. The loaders reject cross-mode catalogs and placements.

At an authenticated ordinary frag fuse terminal, the shared planner reads the
accepted owner-event Character and Ground pose view and the installed mode's
collision frame. Either the source-bound chassis or wheel can qualify overlap.
The current model origin and orientation determine native area 1–6, radial
attenuation, direction and one target-excluding layer-24 material ray. The dose
uses the vehicle channel, zero prevention reductions, current vehicle armor and
existing friendly-fire scaling. This replaces the opted-in generic calculation
that used area zero, authored reductions and three committed-world cover rays.

Field 18, health and the normal Ground lifecycle join the outer recorded owner
candidate. Recorder refusal leaves motion, flight, health and pending queues
unchanged. Typed query failures retain legitimate motion and terminal handling,
record a denial and cannot fall through to the old damage calculation. Existing
Helo frag handling runs independently of this Ground selection under its prior
admission conditions; its numerical correction and recipient qualification are
separate work.

One dose per allocated vehicle per terminal is an explicit bounded replacement
policy even if both roots overlap. Native repeated-callback parity is unverified.
The outgoing vehicle source word remains the explicit invalid sentinel; the
admitted projectile identity is retained in audit and the native vehicle input
source word remains unqualified. Other vehicles' wheel cover keeps its existing
represented-world query behavior.

Original-input projection and scripted Engine replay qualify source behavior.
They do not establish native flight, callback ordering or live-match acceptance.
The maintainer reviews a live match and cuts releases manually.
