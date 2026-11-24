# Manual squad orders

Acquisition, Domination and Sabotage support tagged-object FRAGO orders and map
waypoints from the currently held squad leader. Configure a retail worker with
`--squad-orders /absolute/ORDERS.json` and native leadership
(`--leadership-passives` and `--leadership-role-policy`). Without that
configuration the typed requests are recognized and denied: a worker without
native leadership has no held squad leader (`squad_order_requires_held_squad_leader`),
and one without order rules reports `squad_orders_unconfigured`. The setting applies
to standalone and formation-registry workers; each round gets fresh order state.

Generate the rules with `tools/extraction/project-retail-squad-orders.py` from
the supported original retail rules archive. The projector reads the authored
character gameplay order cooldown, retains original archive/member hashes, and
can check a retained projection with `--check`. Populated rules and original
files remain external to the public checkout. Startup validation loads the rules
before opening logs or listeners and reports `squad_orders_validated`.

Requests contain either a full object ID or XYZ plus a clear bit. The server
derives the actual squad from the authenticated connection and compares its
reservation-bound held leader. A platoon leader or OIC has no squad override.
Both set operations share the native unsigned timestamp gate and its half-second
server allowance. Successful sets stamp server time. Clearing either order and
removing the leader retain that timestamp. The native non-increasing-clock branch
is preserved. Set waypoints use the existing finite position encoder domain;
clear retains the last coordinates.

Tag admission resolves the full ID against current objective, ground vehicle,
helicopter or hospital inventory. Unimplemented target families are unavailable.
It does not invent an enemy-team or objective-state filter. Existing mode
assignment refresh keeps the primary objective separate from the manual tag and
chooses the effective tagged object. Clearing returns to authored fallback order,
subject to the same native refresh gates. Automatic objective changes also
publish the resulting effective tag.

Sabotage has no source gameplay provider, so the order runtime keeps each
squad's manual tag itself. The tag is a replacement policy, not a decoded native
refresh: it stays until the leader clears or replaces it, the leader loses the
role, or the round resets. The Sabotage objective catalog carries no FRAGO
fallback order list, so a cleared tag publishes FFFFFFFF. The squad's FRAGO assignment pairs that tag with
the existing Sabotage primary, the first authored objective that is not Inactive,
Completed or Locked. Kill, support and objective rewards use it with the existing
geometry and admission gates. Sabotage objective rewards take the objective as
their context. Sabotage FRAGO geometry still needs a projector extension for the
Sabotage authored classes; without geometry, rewards keep base XP.

Orders commit only after the event is recorded. Loss or replacement of the held
squad leader clears the waypoint and manual tag with the role reconciliation,
before exposing a replacement leader. It does not clear the Company's other
squads or restart the shared order gate. Reliable type-8 fields13–15 publish the
effective tag, waypoint position and active bit after squad creation. Current
recipients receive changes; late joiners receive retained state. Full queues defer
updates, and failed records change neither baseline nor queued bytes.

Synthetic tests verify codecs, shared timing, held-role admission, native mode
assignment behavior and rollback. Source-derived profile checks establish
configuration and selection, not rendered client behavior. Live FRAGO markers,
waypoint presentation and interactions still require native-match acceptance.
Requests grant no immediate XP. FRAGO bonus parity, persistent leadership
advancement and leader rewards remain separate work. Releases remain manual after
the maintainer's live-match review.
