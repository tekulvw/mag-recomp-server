# Combat reward relationships

An actor's assigned match side and faction are separate. Opposing teams can
share a faction, as in the supported two-sided Suppression configuration.
Combat reward relationships use the actual assigned side rather than the
faction, a fixed attacker/defender number or the mere existence of a player.

## Kills

An accepted lethal hit against a friendly registry bot retains its damage,
native death notification and friendly-kill statistic. It does not earn an
`EnemyKilled` receipt. The opposing-side case earns the existing bot-life
receipt even when the shooter and bot share a faction. Firearm and queued
grenade kills retain the same receipt identity and deduplication.

This change does not add a friendly-kill XP penalty or change damage amounts.
Any further penalty or damage-scaling policy requires its own supported event
rules and external configuration.

## Verification limits

Critical synthetic tests exercise the native hit path for a friendly bot and a
same-faction opponent, including unchanged damage, death notifications,
statistics, the shared reward identity and repeated-hit deduplication. They do
not establish mixed-faction native acceptance or complete the broader healing,
friendly-fire, repair and mounting audit. Those consumers need their own
source-grounded checks. Reward amounts remain in external configuration.
Live-match review and releases remain controlled by the human maintainer.
