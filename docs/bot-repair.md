# Bot repair

World-owned bot engineers repair their side's destroyed structural objectives
(Domination's demolition structures, such as anti-aircraft guns and bunkers)
with the mode's repair tool. They use the same repair submission a player's
tool sends, so the repair rules, timing and replication are the ones human
repairs use. This is a replacement design written for this server.

## Engineers

Bots in the squad role slots listed in `engineer_role_slots` (default `[2]`,
one per squad) carry the mode's repair tool (the catalog's repair kit) in
place of their self-healing item. Every other item in the kit, including the
weapon, is unchanged. A kit without a healing slot stays as it is.

## Duties

An objective needs repair when it is a structural objective whose repair rule
allows repair, and its state is `Completed` (destroyed) or `Repairing`. For
each such objective, the nearest living engineer of the owning side within
`search_radius_m` (80 m) of the objective, not in a supply task, is sent to
it. That duty replaces its squad's lane, as an armed-objective duty does
([bot-armed-objectives.md](bot-armed-objectives.md)), and the engineer walks
to the objective's operator spot inside its authored range. Each engineer
takes at most one duty, and an armed-objective duty wins over a repair.

While the engineer stands still inside that range, it submits one repair per
tool cooldown, with the authority a player's tool would carry:
- its own side;
- a live actor;
- the tool active;
- the unmodified tool rate, because bots own no skills.

The objective's repair runtime then applies the repair, its state transitions
and its replication exactly as for a player. Bots earn no repair XP. A
refused submission is recorded and retried after 2 s. The tool is not shown
equipped on the bot's model, which keeps its weapon in hand.

Attacking bots destroy these structures with demolition teams
([bot-demolition.md](bot-demolition.md)).

An engineer that is the chosen operator of the objective it repairs walks to a
spot inside the objective's repair range.

## Configuration

The population file's `repair` object; omitted fields use the defaults:

```json
{"enabled": true, "engineer_role_slots": [2], "search_radius_m": 80}
```

`"enabled": false` or `"engineer_role_slots": []` keeps every bot on its
default kit and turns repair duties off.

## Evidence

- `bot_repair`: the engineer, its life, the objective, whether the submission
  was accepted and queued (`false` when the queued health already covers the
  repair), and the refusal reason.
- `domination_bot_repair`: the matching record inside the Domination
  transaction.

## Not implemented

- Acquisition structural repair, vehicle repair and hospital (bunker)
  repair by bots.
- Repairing damaged but not destroyed objectives. The repair runtime only
  accepts a destroyed or repairing objective.
