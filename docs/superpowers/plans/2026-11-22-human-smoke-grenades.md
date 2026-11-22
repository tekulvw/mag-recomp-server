# Human starter smoke grenades

Implement the three stock Marksman grenade items as an explicit optional retail
capability, using owner-reported thrown physics as for the existing frags. Keep
original data external and keep releases manual. Live client acceptance remains
separate from offline server checks.

1. Project and validate item/faction/ammo binding, finite inventory, fuse,
   projectile lifetime and independent volume duration from original resources.
   Reject damaging or non-Smoke/Volume strategies. Add synthetic malformed-input
   checks and reproduce the projection against the owner's original files.
2. Add the qualified retail family-5 active/owner/position codec with a 32-object
   pool. Test independent bit decoding, bounds, canonical inactive removal and
   explicit retail scope. Do not invent duration or visual fields.
3. Connect smoke to normal authenticated gear-4 launch and finite grenade supply.
   Reuse flight admission and owner reports; classify smoke before common frag
   damage consumers. Commit the terminal, volume and retained parent only after
   successful recording. A failed allocation/encoding/record leaves the terminal
   retryable. Preserve duplicate and shared projectile slot fences.
4. Retain exploded parent presentation and family-5 volume through the authored
   duration from explosion, including snapshots. Expiry queues parent and volume
   removals; pool reuse waits for queued removal delivery. Reservation/world/round
   teardown removes both. Already exploded smoke can outlive the owner's death;
   unreported flight positions do not fabricate a grounded smoke explosion.
5. Add explicit Acquisition and Domination startup paths and readiness failures.
   Check normal connected launch/fuse/expiry/late join, independent mixed frag and
   smoke terminals, no damage/XP, pool pressure, failed record, source teardown
   and static supply refill with synthetic peers. Run affected checks, strict
   lint/build and public boundary verification, then integrate checked source.

The native family-5 row has no lifetime or ammo property. Projectile replication
independently invokes the client Volume strategy. Client presentation and exact
late-join visual age must follow the original reader evidence; passing wire
fixtures does not establish rendered smoke or occlusion.
