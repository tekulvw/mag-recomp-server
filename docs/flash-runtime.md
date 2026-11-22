# Stock human Flash grenade runtime

Flash is an optional source-bound server capability for the owned retail client.
It is disabled when the configuration omits `flash_grenades` in Acquisition or
`core.flash_grenades` in Domination. The value is a relative or absolute path to
an external `mag_retail_flash_grenades_v1` rules projection. The projector and
source qualification are described in `flash-grenades.md`; neither client assets
nor original program code are included in the server.

The catalog admits the three faction Flash items, their original ammo CRCs,
three finite uses, Timer/Thrown/Debris strategies, hearing-loss flag and zero HP
channels. It checks the existing item/binding/bootstrap/Armory source pins and
the aggregate Flash, facing and material proof digest. Values with native float
semantics are compared at f32 resolution. The unavailable historical Flash skill
is not an equipment gate: normal authenticated current equipped inventory and
Recruit-granted availability govern this route.

Admission reuses gear slot 4, the four common grenade projectile slots, retained
parent creation, duplicate detection, fuse, finite ammo and static resupply.
Launch provenance includes reservation, world, player, allocation, current life
and round. An admitted owner terminal sends one ordinary state-2 Debris terminal
to peers. The locally predicted owner does not receive this remote explosion
again. Reliable parent cleanup and pending-publication fences precede slot reuse.
There is no family-5 volume, HP damage, assist/XP credit, stun state or server
blindness timer.

The bounded authoritative victim route supports current living on-foot human
Characters. It includes the firer and allies, regardless of damage-only temporary
invulnerability. Mounted, downed, retained dead and bot receiver behavior is not
claimed by this subset. Victim overlap uses the source row-6 filter. The radial
and material ray sample uses the current stance-adjusted body anchor; facing
uses the explosion delta from the unadjusted current body origin.

On a qualified clear ray, the duration is 10 seconds only when the normalized
source delta dot normalized yaw-forward is strictly greater than the source
cosine threshold. Equality and degenerate direction choose the 0.1-second weak
pulse. Packed yaw maps 0/256/512/768 to +Z/+X/-Z/-X using the original 1/1024
scale. Both branches multiply by native radial falloff (1 at the center, 0.5 at
10 meters, zero at 40 meters).

A dedicated facing witness is required for the clear-ray route. A fabricated
spawn yaw, TCP presentation update, stale packet, another allocation or prior
life is insufficient. Only an explicitly supplied yaw in a fresh authenticated
current living on-foot UDP owner update can stage the witness. The accepted self yaw can participate in the same terminal candidate; packets and
witness installation both wait for successful recording. Spawn, downing/death and any mount/exit transition
invalidate it; revival does not restore it. A same-event terminal transition
wins over a witness staged from the preceding alive snapshot. A facing-dependent
victim without that witness is explicitly audited as unsupported. A separately
qualified cover obstruction chooses the weak pulse without needing yaw.

Flash cover uses the exact row-24 filter, source-bound static part filters and
current transformed Sequence meshes. Sequence root/child layers come from the
validated exact contact binding, because the retained original meshes do not
carry per-part filters. Excluded bound surfaces skip traversal; included ones
retain authored material costs, strict penetration comparisons, terrain samples
and the shared terminal mesh budget. Existing hospital/transport mesh gathering
and primitive Ground null-material handling are retained. Unknown bindings,
materials and exhausted queries remain errors; they never become a fabricated
weak pulse. Legacy damage and air-strike ray behavior is unchanged.

Within one recorded owner event, qualified durations merge by current recipient
using their maximum before delivery. The native client retains the stronger
visual timer; this bounded replacement policy intentionally combines repeated
fixed camera-shake/rumble restarts in the same event. It does not claim identical
cosmetic restart counts or inter-packet timing. All terminal/victim audits and
the three distinct peer Debris/audio triggers remain separate. Distinct owner
events deliver independently.

The final direct reply batch is checked against the same existing count and
24,000-byte limits as shared delivery before evidence recording. An oversized
mixed batch is denied before terminal, facing, ammunition or history state is
committed, so it cannot fail its initial delivery budget after committing.

Each selected positive duration produces one addressed reliable application
message `0x0200` for the current victim's reservation/world/player. It contains
active=true, sign=false, five whole-magnitude bits and ten fractional bits with
native f32 truncation, MSB first. For example, 10 seconds encodes as
`0200940000`, 0.1 seconds as `0200803300`. This is a direct application payload,
not an ObjectUpdate: it does not enter object-update stamping or history.
The existing authenticated Horizon route enqueues these replies on the
recipient TCP connection. The client owns presentation and expiry; ordinary
projectile replication supplies its separate native hearing effect.

Synthetic Rust checks cover codec boundaries, cardinal facing and equality,
radial/weak selection, authenticated current-life yaw, transition invalidation,
record failures, self/allied targeting, finite ammo/refill, peer-only terminal,
cleanup/reuse, exact contact-layer filtering, material costs and query errors.
Independent startup/replay against external owner-held data remains separate
from live-client visual or behavioral acceptance. Releases remain manual.
