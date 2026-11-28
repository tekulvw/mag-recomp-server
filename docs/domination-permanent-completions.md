# Domination permanent completions

In Domination, a destroyed burn-off tower or cooling tower stays destroyed for
the rest of the match. The defenders cannot retake it, a script cannot reopen
it, and the attackers keep the completion. Capture points (A–H) can still be
retaken as before.

## Which objectives

Each map quadrant has a chain of secure-and-hold objectives. The authored
controller locks the first pair once both are Completed and completes a
proxy. Five seconds later it opens the second pair, which works the same way
and then opens the quadrant's two capture points.

The tower pairs and the capture points use different authored definitions.
The server tells them apart from the map's structural-effect definitions
(`structural_effects` in the Domination gameplay configuration), so no
objective ids are hard-coded. A secure-and-hold objective's completion is
permanent when its definition:

- plays a sequence on entering Completed (index 0 or above), and
- keeps that sequence through every retake state: Retaking, Retaken and
  CancelRetaken all author -2 ("keep the current sequence").

A tower therefore has no authored look for being retaken. A capture point
authors a Retaken sequence, so it keeps its retake. Definitions without a
Completed sequence (proxies and script-driven markers) are not affected.
Structural objectives (bunkers, anti-aircraft, gates, motor pools and so on)
are not secure-and-hold objectives and keep their authored `CanRepair` repair
rule.

The retail Canal, Baku and Unalaska data each yield 16 permanent towers and 8
retakeable capture points.

## Rules

For a permanent objective:

- A Deactivate request from the owning side on a Completed objective is
  refused with `secure_hold_completion_permanent`, for players and bots alike.
  Defending inside the attackers' completion window (Activated →
  Deactivating) is unchanged.
- Defender bot operators do not choose a Completed or Retaking permanent
  objective as an action.
- A controller request for Inactive or Deactivated on a Completed permanent
  objective leaves it Completed. Locking it still works.

So a tower pair locks, and the chain moves on, as soon as both towers have
fallen.

## Loading

`Configuration::mark_permanent_completions` marks the objective catalog from
the configured structural effects before the objective world is built. That
happens both in a match profile and in the single-profile
`with_domination_gameplay` path. Loading refuses a placement token that the
catalog does not bind, and refuses to build a Domination world from an
unmarked catalog (`domination_permanent_completions_unmarked`). The marking is
not part of the catalog file format.

## Acquisition

Acquisition has no secure-and-hold objectives. Its structural objectives
already follow their authored `CanRepair` rule, so this change does not apply
there.
