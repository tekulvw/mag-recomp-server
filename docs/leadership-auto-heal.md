# Living-character leadership AutoHeal

The world scheduler now consumes the effective AutoHeal modifier for initialized,
ready retail characters who are alive and on foot or in a qualified current
APC/Escort seat in an active match. Ground occupants retain their actual team,
current allocation and unique seat checks; gunners require a current grant but
do not need an initialized primary weapon. Existing
leadership inheritance supplies scope, provider range, hold and removal grace.
The ordinary owned Health modifier determines the current maximum HP. No new
resource schema or populated balance table is bundled with the server.

Each reservation, life generation and round owns a separate f32 timer. An absent
or near-zero modifier clears accumulated time. A pulse occurs only when elapsed
time is strictly above 1.5 seconds, consumes one interval and retains excess.
A delayed update performs one pulse. Full health and declined healing consume the
due interval. Life, connection and round turnover discard the prior timer.
AutoRepair keeps its independent timer and vehicle target.
Mounting or dismounting within the supported living scope does not restart the
Character's timer. An unsupported state or invalid association discards it.

A pulse computes a percentage of current maximum HP, caps against missing float
HP and truncates that application to quarter HP. Positive sub-quarter applications
can retain the same HP while still setting Heal6. That application preserves
conditions, hit location, attacker, source object and userdata. Condition bit
`0x02` blocks healing; character spawn invulnerability does not independently
block Heal6. Incapacitated/dead recipients, Hospital/Helo mounts and special
insertion states remain excluded. The independent native model guard's special-state producers are
not represented by this ordinary living path.

Timer, effective leadership cache, life HP, full damage model and field 8 packets
are staged before the event record. Failed recording installs none of them.
Initialized current/loading peers receive the reliable update, and late joins
read the committed damage model through the existing character snapshot. When a
scheduled health update is backlogged, newer event-driven health/life replies join
the same FIFO until it drains; their reliable sequences are assigned at drain time.
A partial drain retains this ordering barrier without dropping or rewriting bytes. Existing
item effects, cooldowns, ordinary XP and healing-credit callbacks are untouched.

The server uses its deduplicated 50 ms world cadence and publishes the committed
pulse in that scheduler transaction. Retail marks the model dirty after generic
character update, with ordinary publication on a later eligible generic update.
Exact interleaving with regen, item healing and native replication is separate
fidelity work. Synthetic Engine checks and private original-build qualification
establish source behavior; native visual/live-match acceptance remains pending.
Release decisions remain manual after that review.
