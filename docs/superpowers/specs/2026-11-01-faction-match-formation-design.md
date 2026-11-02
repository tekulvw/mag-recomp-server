# Faction-aware Sabotage and Acquisition match formation

The human requires all three saved character factions to attack or defend in
both modes. Wait for enough human players, prioritize the longest-waiting
eligible faction, randomly choose its eligible opponent, and independently
randomize roles. Keep other players queued. Characters never change faction.
The defender selects its home map. Each mode has six ordered faction pairings.

Separate lobby queue intent from DME admission. Queueing must not block the MLS
message loop or fabricate an error response as successful waiting. Native queue
update/cancel packets need their original codec contract. Form matches only
from current authenticated human sessions, with the configured minimum and at
least one per selected faction. Bots do not count. Random selection is uniform
over eligible opponents and a separate two-way role draw; inject draws in tests.

Use explicit preloaded source-qualified worker profiles. Before any reservation
or GameInfo, acknowledge an immutable profile/formation selection on the idle
worker. Pin that selection for the cohort; late join can fill its two factions
without rerolling. The third faction waits. New-round selection requires old
members/releases/results to clear. Preserve the current single-worker limit;
this does not claim concurrent multi-world hosting.

Bridge interface: hello advertises capability `match_profiles_v1` and
`match_profiles` rows `{profile_id,game_type,attacking_faction,defending_faction,
maximum_players,team_capacity}`. Selection request is
`{v:2,request_id,op:"select_match_profile",formation_id,profile_id}`.
Success echoes `selected_match` with `{formation_id,profile_id,game_type,
attacking_faction,defending_faction}`. Exact retries are idempotent; active
conflicting selection is rejected. Reserve is closed before a selection.

All catalogs/assets stay external/private. A profile must carry coherent
spawn/objective/collision/vehicle/damage inputs. Existing native identity and
vehicle gaps remain explicit. Keep existing fixed-profile operation unless
formation is explicitly enabled. No service/game/release action is authorized
by passing source checks; the human cuts releases after live-match review.
