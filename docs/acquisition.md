# Acquisition foundation

The maintained spawn projector accepts Arulkum's Acquisition profile with an
explicit opposing attacker:

```sh
python3 tools/extraction/build-retail-spawn-catalog.py \
  --disc "$MAG_RETAIL_DISC" --game-archive "$MAG_RETAIL_GAME" \
  --map mag_voz_final --active 1/0/0 --attacking-faction Valor \
  --output "$MAG_ACQUISITION_PROFILE"
```

Inputs are external original resources; no game data is distributed here. Use
`--check` to reproduce an existing output without rewriting it. Raven is also
an explicit attacker choice; the defending owner cannot attack itself.

The projection retains authored location order, activation, both platoons'
squad bindings and original archive/rule hashes. Active Basic player insertion
and alternate points can be selected. Other spawn kinds remain unsupported,
including active vehicle-user and hospital points. Raw unsupported kind,
category, user class, activation and squad links remain in the source metadata.

The local profile uses maximum64/team32/squad4. Authored maximum/minimum counts
are provenance and do not establish supported capacity. Native pool identities
remain unverified. Bunkers and objectives are disabled in this foundation.

Some authored squad insertion points require APCs. Their unique Basic alternate
is recorded as a temporary local default fallback. **Vehicle spawning is a
required Acquisition milestone; these fallbacks do not complete entry support.**
Vehicle creation, replication and vehicle-backed insertion need their own typed
implementation and correctness evidence. A spawn record is not a vehicle entity.

Retail mode1 now has matching source admission policy in Horizon and Rust, within
the existing64-player maximum. Explicit operator configuration is required; no
running service configuration changes here. Prototype Acquisition remains
unsupported and other prototype modes retain their eight-player bound.

Synthetic checks cover64 pending reservations, four members in each of16 squads,
all64 Ready snapshots, second-platoon spawn assignment, physical respawn at that
squad's configured point, exact packet replay and late-join roster preservation.
The existing generic squad addressing and spawn path already preserve both
platoons; no native field layout is inferred from those checks.

This source foundation does not establish native playability or fulfill required
vehicle spawning. Keep source integration, native review and the maintainer's
manual release action distinct.
