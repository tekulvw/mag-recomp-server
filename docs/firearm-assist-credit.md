# Ordinary firearm assist credit

Firearm assist counters are optional. Pass an explicit external rules file with
`--firearm-assist-rules /absolute/ASSISTS.json` for application20231. Without
that option the feature is disabled. Rules contain schema_version1,
application_id20231 and an unsigned assist_timeout_ms. A zero timeout permits
credit only at the same instant; it does not disable tracking.

The maintained projector consumes the existing neutral retail-rules extraction:

```sh
python3 tools/project-retail-firearm-assist-rules.py \
  --retail-rules /absolute/retail-rules.json \
  --out /absolute/ASSISTS.json \
  --receipt /absolute/ASSISTS.receipt.json
```

Add `--check` to reproduce existing bytes. The projector selects the named
MAGCharacterStats table, verifies the numeric value against its exact source
literal, and emits a separate input/member provenance receipt. Original files
and extracted catalogs remain operator-supplied private inputs.

Each current victim allocation/life has five contributor entries. An admitted
ordinary firearm hit on a living opposing Character records its attacker even
if damage absorption leaves HP unchanged. A repeated contributor refreshes its
expiry. New contributors fill the first empty entry, then replace the earliest
expiry, with first-entry preference for ties. Invalid, self and friendly hits
never create a contribution.

At the first ordinary-firearm lethal collapse, eligible current contributors other than the
killer gain one assist (stat11). Incapacitation settles that collapse; finishing
or bleed-out cannot settle it again. A revived Character begins a fresh ledger.
The contributing allocation can respawn and keep eligibility, while a different
human admission or bot allocation in its slot cannot inherit old credit.

Deadlines use wide server-monotonic millisecond stamps and include the expiry instant.
This replaces the original wrapping network-time arithmetic, preserving normal
interval behavior without carrying its wraparound anomalies. The timeout comes
from disk; the implementation embeds no gameplay default.

Contribution/counter plans join the existing recorded damage transactions.
Recorder refusal preserves their prior state. Human assists update the existing match counters and use coalesced stat11-only
rows in the bounded drain. Selection reads the latest total and preserves the
queued creation prefix, without waiting for unrelated later traffic. Bot assists join the creation-fenced,
unsequenced totals stream, retaining reply budgets and reliable FIFO ordering.

This increment covers ordinary on-foot firearms and assist counters. Assist XP,
other damage sources, native scoreboard acceptance and bot final-results rows
require their own qualification. Existing legitimate kill/death/XP behavior
remains intact. Releases require the maintainer's live-match review and manual
action.
